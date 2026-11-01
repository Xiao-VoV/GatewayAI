//! 上游执行器：候选/账号选择 + 2 次尝试 + 失败分类（可重试/不可重试）+ 错误清洗。
//!
//! 设计：proxy 技术设计 §3.4/§5.1/§5.2。账号选择失败不消耗 attempt 次数；
//! attempt 2 优先下一候选，候选耗尽再同候选换账号；`fallback` = 最终候选序号 > 0。

use std::collections::HashMap;

use bytes::Bytes;
use uuid::Uuid;

use crate::modules::metering::model::UsageStatus;
use crate::modules::providers::service::{self as providers, SelectAccountError};
use crate::platform::error::AppError;
use crate::platform::state::AppState;

use super::adapters::chat::ChatRequest;
use super::adapters::gemini::GeminiRequest;
use super::adapters::messages::MessagesRequest;
use super::adapters::responses::ResponsesRequest;
use super::error::{ProxyError, model_unavailable, upstream_protocol};
use super::route::PlannedTarget;
use super::settings::ProxySettings;
use super::upstream::{self, HttpFailure, JsonUpstream, StreamUpstream, UpstreamSpec, map_failure};

/// 已按前端协议准备好的请求（适配器只构造一次，执行器按目标替换 model）。
pub enum PreparedBody<'a> {
    Chat(&'a ChatRequest),
    Messages(&'a MessagesRequest),
    Responses(&'a ResponsesRequest),
    Gemini(&'a GeminiRequest),
}

impl PreparedBody<'_> {
    fn prepare(&self, target: &PlannedTarget) -> Result<Bytes, ProxyError> {
        match self {
            Self::Chat(request) => super::adapters::chat::prepare_upstream_body(
                request,
                &target.upstream_model,
                &target.capabilities,
            )
            .map_err(ProxyError::from),
            Self::Messages(request) => super::adapters::messages::prepare_upstream_body(
                request,
                &target.upstream_model,
                &target.capabilities,
            )
            .map_err(ProxyError::from),
            Self::Responses(request) => super::adapters::responses::prepare_upstream_body(
                request,
                &target.upstream_model,
                &target.capabilities,
            )
            .map_err(ProxyError::from),
            Self::Gemini(request) => super::adapters::gemini::prepare_upstream_body(
                request,
                &target.upstream_model,
                &target.capabilities,
            )
            .map_err(ProxyError::from),
        }
    }
}

pub struct JsonSuccess {
    pub target: PlannedTarget,
    pub fallback: bool,
    pub attempt_count: i32,
    pub upstream: JsonUpstream,
}

pub struct StreamSuccess {
    pub target: PlannedTarget,
    pub fallback: bool,
    pub attempt_count: i32,
    pub upstream: StreamUpstream,
}

pub struct ExecutionFailure {
    pub error: ProxyError,
    pub target: Option<PlannedTarget>,
    pub fallback: bool,
    pub attempt_count: i32,
    pub usage_status: UsageStatus,
}

#[allow(clippy::result_large_err)]
pub async fn execute_json(
    state: &AppState,
    settings: &ProxySettings,
    targets: &[PlannedTarget],
    prepared: &PreparedBody<'_>,
) -> Result<JsonSuccess, ExecutionFailure> {
    match run(state, settings, targets, prepared, false).await? {
        RunPayload::Json {
            target,
            fallback,
            attempt_count,
            upstream,
        } => Ok(JsonSuccess {
            target,
            fallback,
            attempt_count,
            upstream,
        }),
        RunPayload::Stream { .. } => unreachable!("json executor returned stream payload"),
    }
}

#[allow(clippy::result_large_err)]
pub async fn execute_stream(
    state: &AppState,
    settings: &ProxySettings,
    targets: &[PlannedTarget],
    prepared: &PreparedBody<'_>,
) -> Result<StreamSuccess, ExecutionFailure> {
    match run(state, settings, targets, prepared, true).await? {
        RunPayload::Stream {
            target,
            fallback,
            attempt_count,
            upstream,
        } => Ok(StreamSuccess {
            target,
            fallback,
            attempt_count,
            upstream,
        }),
        RunPayload::Json { .. } => unreachable!("stream executor returned json payload"),
    }
}

enum RunPayload {
    Json {
        target: PlannedTarget,
        fallback: bool,
        attempt_count: i32,
        upstream: JsonUpstream,
    },
    Stream {
        target: PlannedTarget,
        fallback: bool,
        attempt_count: i32,
        upstream: StreamUpstream,
    },
}

struct AttemptFailureRecord {
    failure: HttpFailure,
    target: PlannedTarget,
    fallback: bool,
    attempt_count: i32,
}

#[allow(clippy::result_large_err)]
async fn run(
    state: &AppState,
    settings: &ProxySettings,
    targets: &[PlannedTarget],
    prepared: &PreparedBody<'_>,
    is_stream: bool,
) -> Result<RunPayload, ExecutionFailure> {
    let mut tried: HashMap<usize, Vec<Uuid>> = HashMap::new();
    let mut last_index: Option<usize> = None;
    let mut last_failure: Option<AttemptFailureRecord> = None;
    let mut attempt_count: i32 = 0;
    let max_attempts = i32::from(settings.max_attempts.max(1));

    while attempt_count < max_attempts {
        let choice = choose_target(state, targets, &tried, last_index).await;
        let (target_index, target, account_id, credential) = match choice {
            TargetChoice::Found {
                index,
                target,
                account_id,
                credential,
            } => (index, target, account_id, credential),
            TargetChoice::None => break,
            TargetChoice::Fatal(error) => {
                return Err(ExecutionFailure {
                    error,
                    target: last_failure.as_ref().map(|record| record.target.clone()),
                    fallback: last_failure
                        .as_ref()
                        .map(|record| record.fallback)
                        .unwrap_or(false),
                    attempt_count: attempt_count.max(1),
                    usage_status: UsageStatus::GatewayError,
                });
            }
        };
        let fallback = target_index > 0;
        let body = match prepared.prepare(&target) {
            Ok(body) => body,
            Err(error) => {
                return Err(ExecutionFailure {
                    error,
                    target: Some(target),
                    fallback,
                    attempt_count: attempt_count.max(1),
                    usage_status: UsageStatus::GatewayError,
                });
            }
        };
        attempt_count += 1;
        let spec = UpstreamSpec {
            url: target.completions_url(),
            credential,
            body,
            is_stream,
        };
        let outcome = if is_stream {
            upstream::execute_stream(state, settings, &target, &spec)
                .await
                .map(|upstream| RunPayload::Stream {
                    target: target.clone(),
                    fallback,
                    attempt_count,
                    upstream,
                })
        } else {
            upstream::execute_json(state, settings, &target, &spec)
                .await
                .map(|upstream| RunPayload::Json {
                    target: target.clone(),
                    fallback,
                    attempt_count,
                    upstream,
                })
        };
        match outcome {
            Ok(payload) => return Ok(payload),
            Err(failure) => {
                let retryable = failure.retryable;
                let record = AttemptFailureRecord {
                    failure,
                    target,
                    fallback,
                    attempt_count,
                };
                if retryable && attempt_count < max_attempts {
                    tried.entry(target_index).or_default().push(account_id);
                    last_index = Some(target_index);
                    last_failure = Some(record);
                    continue;
                }
                return Err(map_http_failure(record));
            }
        }
    }

    // 没有发出任何上游请求（无候选/无账号/provider 禁用）。
    if let Some(record) = last_failure {
        return Err(map_http_failure(record));
    }
    Err(ExecutionFailure {
        error: model_unavailable(),
        target: None,
        fallback: false,
        attempt_count: attempt_count.max(1),
        usage_status: UsageStatus::GatewayError,
    })
}

fn map_http_failure(record: AttemptFailureRecord) -> ExecutionFailure {
    let usage_status = if record.failure.kind == upstream::FailureKind::Timeout {
        UsageStatus::Timeout
    } else {
        UsageStatus::UpstreamError
    };
    let error = map_failure(record.failure);
    ExecutionFailure {
        error: error.into(),
        target: Some(record.target),
        fallback: record.fallback,
        attempt_count: record.attempt_count,
        usage_status,
    }
}

#[allow(clippy::large_enum_variant)]
enum TargetChoice {
    Found {
        index: usize,
        target: PlannedTarget,
        account_id: Uuid,
        credential: String,
    },
    None,
    Fatal(ProxyError),
}

/// 候选/账号选择顺序：
/// - attempt 1：`0..len`；
/// - 后续：`last+1..len` → 同候选换账号（`tried` 排除）→ `0..last` 兜底。
async fn choose_target(
    state: &AppState,
    targets: &[PlannedTarget],
    tried: &HashMap<usize, Vec<Uuid>>,
    last_index: Option<usize>,
) -> TargetChoice {
    let order: Vec<usize> = match last_index {
        None => (0..targets.len()).collect(),
        Some(last) => {
            let mut order: Vec<usize> = ((last + 1)..targets.len()).collect();
            order.push(last);
            order.extend(0..last);
            order
        }
    };
    for index in order {
        let Some(target) = targets.get(index) else {
            continue;
        };
        // catalog 固定账号：失败不能做账号级重试（换账号会偏离部署语义），
        // attempt 2 不应再尝试同一候选。
        if Some(index) == last_index && target.provider_account_id.is_some() {
            continue;
        }
        let excluded = tried.get(&index).cloned().unwrap_or_default();
        match select_credential(state, target, &excluded).await {
            Ok((account_id, credential)) => {
                return TargetChoice::Found {
                    index,
                    target: target.clone(),
                    account_id,
                    credential,
                };
            }
            Err(SelectionError::Unavailable) => continue,
            Err(SelectionError::Fatal(error)) => return TargetChoice::Fatal(error),
        }
    }
    TargetChoice::None
}

enum SelectionError {
    Unavailable,
    Fatal(ProxyError),
}

async fn select_credential(
    state: &AppState,
    target: &PlannedTarget,
    excluded: &[Uuid],
) -> Result<(Uuid, String), SelectionError> {
    if target.provider_kind != "openai_compatible" {
        return Err(SelectionError::Fatal(upstream_protocol(
            "S4 仅支持 openai_compatible 上游",
        )));
    }
    // v0.1.x catalog 固定账号：直接解密（不做账号级重试）。
    if let Some(account_id) = target.provider_account_id {
        return match providers::decrypt_credential(state, account_id).await {
            Ok(secret) => Ok((account_id, secret.expose().to_string())),
            Err(AppError::NotFound { .. }) => Err(SelectionError::Unavailable),
            Err(error) => Err(SelectionError::Fatal(error.into())),
        };
    }
    match providers::select_account(state, target.provider_id, excluded).await {
        Ok(account) => Ok((account.account_id, account.credential.expose().to_string())),
        Err(SelectAccountError::ProviderUnavailable | SelectAccountError::NoAvailableAccount) => {
            Err(SelectionError::Unavailable)
        }
        Err(SelectAccountError::CredentialUnavailable) => {
            tracing::error!(
                provider_id = %target.provider_id,
                "upstream credential unavailable; check GW_MASTER_KEY/GW_MASTER_KEY_FILE"
            );
            Err(SelectionError::Fatal(ProxyError::from(
                AppError::bad_gateway("upstream_error", "上游凭证暂时不可用，请联系管理员", None),
            )))
        }
        Err(SelectAccountError::DatabaseUnavailable) => Err(SelectionError::Fatal(
            AppError::database_unavailable().into(),
        )),
    }
}
