//! providers 用例：CRUD、删除规则、连通性测试与内部取凭证接口。

use std::collections::BTreeMap;

use chrono::Utc;
use uuid::Uuid;

use crate::platform::{error::AppError, state::AppState};

use super::{
    connectivity, crypto,
    model::{
        AccountList, CreateAccountRequest, CreateModelRequest, CreateProviderRequest,
        KIND_OPENAI_COMPATIBLE, MAX_ACCOUNTS, MAX_MODELS, ModelList, PatchAccountRequest,
        PatchModelRequest, PatchProviderRequest, Provider, ProviderAccount, ProviderDetail,
        ProviderPage, TestError, TestRequest, TestResult, UpstreamModel,
    },
    repo, validate,
};

const PROVIDER_NOT_FOUND: &str = "provider_not_found";
const ACCOUNT_NOT_FOUND: &str = "account_not_found";
const MODEL_NOT_FOUND: &str = "upstream_model_not_found";

pub fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

fn provider_not_found() -> AppError {
    AppError::not_found(PROVIDER_NOT_FOUND, "供应商不存在")
}

fn account_not_found() -> AppError {
    AppError::not_found(ACCOUNT_NOT_FOUND, "渠道账号不存在")
}

fn model_not_found() -> AppError {
    AppError::not_found(MODEL_NOT_FOUND, "上游模型不存在")
}

fn to_provider(row: repo::ProviderRow) -> Provider {
    Provider {
        id: row.id,
        name: row.name,
        kind: row.kind,
        base_url: row.base_url,
        description: row.description,
        status: row.status,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn to_account(row: repo::AccountRow) -> ProviderAccount {
    ProviderAccount {
        id: row.id,
        provider_id: row.provider_id,
        name: row.name,
        region: row.region,
        api_key_hint: row.credential_hint,
        weight: row.weight,
        priority: row.priority,
        rpm_limit: row.rpm_limit,
        tpm_limit: row.tpm_limit,
        health: row.health,
        status: row.status,
        last_checked_at: row.last_checked_at,
        last_error: row.last_error,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn to_model(row: repo::ModelRow) -> UpstreamModel {
    let capabilities = row
        .capabilities
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    UpstreamModel {
        id: row.id,
        provider_id: row.provider_id,
        name: row.name,
        canonical: row.canonical,
        capabilities,
        context_window: row.context_window,
        max_output: row.max_output,
        price_input: row.price_input,
        price_output: row.price_output,
        currency: row.currency,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub fn pagination(limit: Option<i64>, offset: Option<i64>) -> Result<(i64, i64), AppError> {
    let limit = limit.unwrap_or(50);
    if !(1..=200).contains(&limit) {
        return Err(AppError::validation_field("limit", "limit 必须在 1–200"));
    }
    let offset = offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::validation_field("offset", "offset 必须 ≥0"));
    }
    Ok((limit, offset))
}

fn status_filter(status: Option<&str>) -> Result<Option<String>, AppError> {
    match status {
        None => Ok(None),
        Some(value) => validate::validate_status(value)
            .map(Some)
            .map_err(|message| AppError::validation_field("status", message)),
    }
}

fn kind_filter(kind: Option<&str>) -> Result<Option<String>, AppError> {
    match kind {
        None => Ok(None),
        Some(value) if value == KIND_OPENAI_COMPATIBLE => Ok(Some(value.to_string())),
        Some(_) => Err(AppError::bad_request(
            "unsupported_kind",
            "S2 仅支持 openai_compatible 供应商",
        )),
    }
}

pub async fn list_providers(
    state: &AppState,
    status: Option<&str>,
    kind: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<ProviderPage, AppError> {
    let pool = db(state)?;
    let status = status_filter(status)?;
    let kind = kind_filter(kind)?;
    let (limit, offset) = pagination(limit, offset)?;
    let total = repo::count_providers(pool, status.as_deref(), kind.as_deref()).await?;
    let rows =
        repo::list_providers(pool, status.as_deref(), kind.as_deref(), limit, offset).await?;
    Ok(ProviderPage {
        items: rows.into_iter().map(to_provider).collect(),
        total,
        limit,
        offset,
    })
}

pub async fn get_provider(state: &AppState, provider_id: Uuid) -> Result<ProviderDetail, AppError> {
    let pool = db(state)?;
    let provider = repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    let counts = repo::provider_counts(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    let routable = provider.status == "active" && counts.active_account_count > 0;
    Ok(ProviderDetail {
        provider: to_provider(provider),
        account_count: counts.account_count,
        active_account_count: counts.active_account_count,
        upstream_model_count: counts.upstream_model_count,
        model_alias_count: counts.model_alias_count,
        routable,
    })
}

struct ValidatedAccount {
    name: String,
    api_key: String,
    region: String,
    weight: i32,
    priority: i32,
    rpm_limit: Option<i32>,
    tpm_limit: Option<i64>,
}

fn validate_new_account(
    request: &CreateAccountRequest,
    prefix: &str,
    errors: &mut BTreeMap<String, String>,
) -> Option<ValidatedAccount> {
    let name = match validate::validate_name(&request.name) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert(format!("{prefix}name"), message);
            None
        }
    };
    let api_key = match validate::validate_api_key(&request.api_key) {
        Ok(()) => Some(request.api_key.clone()),
        Err(message) => {
            errors.insert(format!("{prefix}api_key"), message);
            None
        }
    };
    let region = match validate::validate_region(&request.region) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert(format!("{prefix}region"), message);
            None
        }
    };
    let weight = match validate::validate_weight(request.weight) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert(format!("{prefix}weight"), message);
            None
        }
    };
    let priority = match validate::validate_priority(request.priority) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert(format!("{prefix}priority"), message);
            None
        }
    };
    let rpm_limit = match validate::validate_rpm_limit(request.rpm_limit) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert(format!("{prefix}rpm_limit"), message);
            None
        }
    };
    let tpm_limit = match validate::validate_tpm_limit(request.tpm_limit) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert(format!("{prefix}tpm_limit"), message);
            None
        }
    };

    Some(ValidatedAccount {
        name: name?,
        api_key: api_key?,
        region: region?,
        weight: weight?,
        priority: priority?,
        rpm_limit: rpm_limit?,
        tpm_limit: tpm_limit?,
    })
}

pub async fn create_provider(
    state: &AppState,
    request: CreateProviderRequest,
) -> Result<(Provider, ProviderAccount), AppError> {
    validate::validate_kind(&request.kind)
        .map_err(|message| AppError::bad_request("unsupported_kind", message))?;

    let mut errors = BTreeMap::new();
    let name = match validate::validate_name(&request.name) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("name".to_string(), message);
            None
        }
    };
    let base_url = match validate::validate_base_url(&request.base_url) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("base_url".to_string(), message);
            None
        }
    };
    let description = match validate::validate_description(&request.description) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("description".to_string(), message);
            None
        }
    };
    let account = validate_new_account(&request.account, "account.", &mut errors);
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let (name, base_url, description, account) = (
        name.ok_or_else(|| AppError::validation_field("name", "名称不合法"))?,
        base_url.ok_or_else(|| AppError::validation_field("base_url", "base_url 不合法"))?,
        description.unwrap_or_default(),
        account.ok_or_else(|| AppError::validation_field("account", "账号字段不合法"))?,
    );

    let provider_id = Uuid::new_v4();
    let account_id = Uuid::new_v4();
    let sealed = crypto::seal_credential(state.crypto.as_ref(), account_id, &account.api_key)?;
    let hint = crypto::mask_api_key(&account.api_key);

    let pool = db(state)?;
    let (provider_row, account_row) = repo::create_provider_with_account(
        pool,
        repo::NewProvider {
            id: provider_id,
            name,
            kind: KIND_OPENAI_COMPATIBLE.to_string(),
            base_url,
            description,
        },
        repo::NewAccount {
            id: account_id,
            name: account.name,
            region: account.region,
            credential_encrypted: sealed,
            credential_hint: hint,
            weight: account.weight,
            priority: account.priority,
            rpm_limit: account.rpm_limit,
            tpm_limit: account.tpm_limit,
        },
    )
    .await?;

    Ok((to_provider(provider_row), to_account(account_row)))
}

pub async fn update_provider(
    state: &AppState,
    provider_id: Uuid,
    request: PatchProviderRequest,
) -> Result<Provider, AppError> {
    if request.name.is_none()
        && request.base_url.is_none()
        && request.description.is_none()
        && request.status.is_none()
    {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }
    let mut errors = BTreeMap::new();
    let name = request
        .name
        .map(|value| match validate::validate_name(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("name".to_string(), message);
                None
            }
        });
    let base_url = request
        .base_url
        .map(|value| match validate::validate_base_url(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("base_url".to_string(), message);
                None
            }
        });
    let description =
        request
            .description
            .map(|value| match validate::validate_description(&value) {
                Ok(value) => Some(value),
                Err(message) => {
                    errors.insert("description".to_string(), message);
                    None
                }
            });
    let status = request
        .status
        .map(|value| match validate::validate_status(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("status".to_string(), message);
                None
            }
        });
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let pool = db(state)?;
    let row = repo::update_provider(
        pool,
        provider_id,
        repo::ProviderPatch {
            name: name.flatten(),
            base_url: base_url.flatten(),
            description: description.flatten(),
            status: status.flatten(),
        },
    )
    .await?
    .ok_or_else(provider_not_found)?;
    Ok(to_provider(row))
}

pub async fn delete_provider(state: &AppState, provider_id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    let counts = repo::provider_counts(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    if counts.model_alias_count > 0 || counts.deployment_count > 0 {
        let references = BTreeMap::from([
            ("accounts".to_string(), counts.account_count as u64),
            (
                "upstream_models".to_string(),
                counts.upstream_model_count as u64,
            ),
            ("model_aliases".to_string(), counts.model_alias_count as u64),
            ("deployments".to_string(), counts.deployment_count as u64),
        ]);
        return Err(AppError::conflict(
            "provider_in_use",
            format!(
                "供应商仍被 {} 个模型别名、{} 个部署引用，不能删除；请先解除引用或禁用",
                counts.model_alias_count, counts.deployment_count
            ),
            Some(references),
        ));
    }
    let deleted = repo::delete_provider(pool, provider_id).await?;
    if !deleted {
        return Err(provider_not_found());
    }
    Ok(())
}

pub async fn list_accounts(state: &AppState, provider_id: Uuid) -> Result<AccountList, AppError> {
    let pool = db(state)?;
    repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    let rows = repo::list_accounts(pool, provider_id).await?;
    if rows.len() > MAX_ACCOUNTS {
        return Err(AppError::bad_request(
            "too_many_items",
            format!("账号数量超过上限 {MAX_ACCOUNTS}，后续版本将支持分页"),
        ));
    }
    Ok(AccountList {
        items: rows.into_iter().map(to_account).collect(),
    })
}

pub async fn create_account(
    state: &AppState,
    provider_id: Uuid,
    request: CreateAccountRequest,
) -> Result<ProviderAccount, AppError> {
    let mut errors = BTreeMap::new();
    let account = validate_new_account(&request, "", &mut errors);
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let account = account.ok_or_else(|| AppError::validation_field("name", "账号字段不合法"))?;

    let pool = db(state)?;
    repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;

    let account_id = Uuid::new_v4();
    let sealed = crypto::seal_credential(state.crypto.as_ref(), account_id, &account.api_key)?;
    let hint = crypto::mask_api_key(&account.api_key);
    let row = repo::insert_account(
        pool,
        provider_id,
        repo::NewAccount {
            id: account_id,
            name: account.name,
            region: account.region,
            credential_encrypted: sealed,
            credential_hint: hint,
            weight: account.weight,
            priority: account.priority,
            rpm_limit: account.rpm_limit,
            tpm_limit: account.tpm_limit,
        },
    )
    .await?;
    Ok(to_account(row))
}

pub async fn update_account(
    state: &AppState,
    provider_id: Uuid,
    account_id: Uuid,
    request: PatchAccountRequest,
) -> Result<ProviderAccount, AppError> {
    if request.name.is_none()
        && request.api_key.is_none()
        && request.region.is_none()
        && request.weight.is_none()
        && request.priority.is_none()
        && request.rpm_limit.is_none()
        && request.tpm_limit.is_none()
        && request.status.is_none()
    {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }

    let mut errors = BTreeMap::new();
    let name = request
        .name
        .map(|value| match validate::validate_name(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("name".to_string(), message);
                None
            }
        });
    let region = request
        .region
        .map(|value| match validate::validate_region(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("region".to_string(), message);
                None
            }
        });
    let weight = request
        .weight
        .map(|value| validate::validate_weight(Some(value)));
    let priority = request
        .priority
        .map(|value| validate::validate_priority(Some(value)));
    let rpm_limit = request
        .rpm_limit
        .map(|value| validate::validate_rpm_limit(Some(value)).map(|_| value));
    let tpm_limit = request
        .tpm_limit
        .map(|value| validate::validate_tpm_limit(Some(value)).map(|_| value));
    let status = request
        .status
        .map(|value| match validate::validate_status(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("status".to_string(), message);
                None
            }
        });
    let mut api_key = None;
    if let Some(value) = request.api_key {
        match validate::validate_api_key(&value) {
            Ok(()) => api_key = Some(value),
            Err(message) => {
                errors.insert("api_key".to_string(), message);
            }
        }
    }
    let weight = weight.and_then(|result| match result {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("weight".to_string(), message);
            None
        }
    });
    let priority = priority.and_then(|result| match result {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("priority".to_string(), message);
            None
        }
    });
    let rpm_limit = rpm_limit.and_then(|result| match result {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("rpm_limit".to_string(), message);
            None
        }
    });
    let tpm_limit = tpm_limit.and_then(|result| match result {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("tpm_limit".to_string(), message);
            None
        }
    });
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let (sealed, hint) = match api_key.as_deref() {
        Some(value) => (
            Some(crypto::seal_credential(
                state.crypto.as_ref(),
                account_id,
                value,
            )?),
            Some(crypto::mask_api_key(value)),
        ),
        None => (None, None),
    };

    let pool = db(state)?;
    let row = repo::update_account(
        pool,
        provider_id,
        account_id,
        repo::AccountPatch {
            name: name.flatten(),
            region: region.flatten(),
            credential_encrypted: sealed,
            credential_hint: hint,
            weight,
            priority,
            rpm_limit,
            tpm_limit,
            status: status.flatten(),
        },
    )
    .await?
    .ok_or_else(account_not_found)?;
    Ok(to_account(row))
}

pub async fn delete_account(
    state: &AppState,
    provider_id: Uuid,
    account_id: Uuid,
) -> Result<(), AppError> {
    let pool = db(state)?;
    let deployments = repo::deployment_count_for_account(pool, account_id).await?;
    if deployments > 0 {
        let references = BTreeMap::from([("deployments".to_string(), deployments as u64)]);
        return Err(AppError::conflict(
            "provider_account_in_use",
            format!("渠道账号仍被 {deployments} 个部署引用，不能删除；请先移除部署"),
            Some(references),
        ));
    }
    let deleted = repo::delete_account(pool, provider_id, account_id).await?;
    if !deleted {
        return Err(account_not_found());
    }
    Ok(())
}

pub async fn list_models(state: &AppState, provider_id: Uuid) -> Result<ModelList, AppError> {
    let pool = db(state)?;
    repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    let rows = repo::list_models(pool, provider_id).await?;
    if rows.len() > MAX_MODELS {
        return Err(AppError::bad_request(
            "too_many_items",
            format!("上游模型数量超过上限 {MAX_MODELS}，后续版本将支持分页"),
        ));
    }
    Ok(ModelList {
        items: rows.into_iter().map(to_model).collect(),
    })
}

pub async fn create_model(
    state: &AppState,
    provider_id: Uuid,
    request: CreateModelRequest,
) -> Result<UpstreamModel, AppError> {
    let mut errors = BTreeMap::new();
    let name = match validate::validate_model_name(&request.name) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("name".to_string(), message);
            None
        }
    };
    let canonical = match validate::validate_canonical(&request.canonical) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("canonical".to_string(), message);
            None
        }
    };
    let capabilities = match validate::validate_capabilities(&request.capabilities) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("capabilities".to_string(), message);
            None
        }
    };
    let context_window = match validate::validate_window(request.context_window, "context_window") {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("context_window".to_string(), message);
            None
        }
    };
    let max_output = match validate::validate_window(request.max_output, "max_output") {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("max_output".to_string(), message);
            None
        }
    };
    let price_input = match validate::validate_price(request.price_input, "price_input") {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("price_input".to_string(), message);
            None
        }
    };
    let price_output = match validate::validate_price(request.price_output, "price_output") {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("price_output".to_string(), message);
            None
        }
    };
    let currency = match validate::validate_currency(request.currency.as_deref().unwrap_or("CNY")) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.insert("currency".to_string(), message);
            None
        }
    };
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let pool = db(state)?;
    repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;

    let capabilities = capabilities.unwrap_or_default();
    let row = repo::insert_model(
        pool,
        provider_id,
        repo::NewModel {
            id: Uuid::new_v4(),
            name: name.ok_or_else(|| AppError::validation_field("name", "模型名不合法"))?,
            canonical: canonical
                .ok_or_else(|| AppError::validation_field("canonical", "canonical 不合法"))?,
            capabilities: serde_json::to_value(capabilities).map_err(AppError::internal)?,
            context_window: context_window.unwrap_or_default(),
            max_output: max_output.unwrap_or_default(),
            price_input: price_input.unwrap_or_default(),
            price_output: price_output.unwrap_or_default(),
            currency: currency
                .ok_or_else(|| AppError::validation_field("currency", "币种不合法"))?,
        },
    )
    .await?;
    Ok(to_model(row))
}

pub async fn update_model(
    state: &AppState,
    provider_id: Uuid,
    model_id: Uuid,
    request: PatchModelRequest,
) -> Result<UpstreamModel, AppError> {
    if request.name.is_none()
        && request.canonical.is_none()
        && request.capabilities.is_none()
        && request.context_window.is_none()
        && request.max_output.is_none()
        && request.price_input.is_none()
        && request.price_output.is_none()
        && request.currency.is_none()
    {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }

    let mut errors = BTreeMap::new();
    let name = request
        .name
        .map(|value| match validate::validate_model_name(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("name".to_string(), message);
                None
            }
        });
    let canonical = request
        .canonical
        .map(|value| match validate::validate_canonical(&value) {
            Ok(value) => Some(value),
            Err(message) => {
                errors.insert("canonical".to_string(), message);
                None
            }
        });
    let capabilities = request.capabilities.map(|value| {
        validate::validate_capabilities(&value)
            .map(|value| serde_json::to_value(value).unwrap_or_default())
    });
    let context_window = request
        .context_window
        .map(|value| validate::validate_window(Some(value), "context_window"));
    let max_output = request
        .max_output
        .map(|value| validate::validate_window(Some(value), "max_output"));
    let price_input = request
        .price_input
        .map(|value| validate::validate_price(Some(value), "price_input"));
    let price_output = request
        .price_output
        .map(|value| validate::validate_price(Some(value), "price_output"));
    let currency = request
        .currency
        .map(|value| validate::validate_currency(&value));
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let context_window = match context_window {
        Some(Ok(value)) => value,
        Some(Err(message)) => {
            errors.insert("context_window".to_string(), message);
            None
        }
        None => None,
    };
    let max_output = match max_output {
        Some(Ok(value)) => value,
        Some(Err(message)) => {
            errors.insert("max_output".to_string(), message);
            None
        }
        None => None,
    };
    let price_input = match price_input {
        Some(Ok(value)) => value,
        Some(Err(message)) => {
            errors.insert("price_input".to_string(), message);
            None
        }
        None => None,
    };
    let price_output = match price_output {
        Some(Ok(value)) => value,
        Some(Err(message)) => {
            errors.insert("price_output".to_string(), message);
            None
        }
        None => None,
    };
    let capabilities = match capabilities {
        Some(Ok(value)) => Some(value),
        Some(Err(message)) => {
            errors.insert("capabilities".to_string(), message);
            None
        }
        None => None,
    };
    let currency = match currency {
        Some(Ok(value)) => Some(value),
        Some(Err(message)) => {
            errors.insert("currency".to_string(), message);
            None
        }
        None => None,
    };
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let pool = db(state)?;
    let row = repo::update_model(
        pool,
        provider_id,
        model_id,
        repo::ModelPatch {
            name: name.flatten(),
            canonical: canonical.flatten(),
            capabilities,
            context_window,
            max_output,
            price_input,
            price_output,
            currency,
        },
    )
    .await?
    .ok_or_else(model_not_found)?;
    Ok(to_model(row))
}

pub async fn delete_model(
    state: &AppState,
    provider_id: Uuid,
    model_id: Uuid,
) -> Result<(), AppError> {
    let pool = db(state)?;
    let aliases = repo::model_alias_count(pool, model_id).await?;
    let deployments = repo::deployment_count_for_model(pool, model_id).await?;
    if aliases > 0 || deployments > 0 {
        let references = BTreeMap::from([
            ("model_aliases".to_string(), aliases as u64),
            ("deployments".to_string(), deployments as u64),
        ]);
        return Err(AppError::conflict(
            "upstream_model_in_use",
            format!("上游模型仍被 {aliases} 个模型别名、{deployments} 个部署引用，不能删除"),
            Some(references),
        ));
    }
    let deleted = repo::delete_model(pool, provider_id, model_id).await?;
    if !deleted {
        return Err(model_not_found());
    }
    Ok(())
}

pub async fn test_provider(
    state: &AppState,
    provider_id: Uuid,
    request: TestRequest,
) -> Result<TestResult, AppError> {
    let pool = db(state)?;
    let provider = repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;

    let account = repo::account_for_test(pool, provider_id, request.account_id).await?;
    let Some(account) = account else {
        if request.account_id.is_some() {
            return Err(account_not_found());
        }
        return Ok(TestResult {
            ok: false,
            latency_ms: 0,
            checked_at: Utc::now(),
            account_id: None,
            http_status: None,
            error: Some(TestError {
                code: "no_account".to_string(),
                message: "该供应商没有可用的 active 账号".to_string(),
            }),
        });
    };

    let secret = crypto::open_credential(
        state.crypto.as_ref(),
        account.id,
        &account.credential_encrypted,
    )?;
    let outcome = connectivity::probe(
        &provider.base_url,
        secret.expose(),
        request.model.as_deref(),
    )
    .await;

    if let Err(error) = repo::update_health(
        pool,
        account.id,
        outcome.ok,
        outcome.error_code.filter(|_| !outcome.ok),
    )
    .await
    {
        tracing::warn!(error = %error, "failed to update provider account health");
    }

    let error = match (outcome.error_code, outcome.error_message) {
        (Some(code), message) => Some(TestError {
            code: code.to_string(),
            message: message.unwrap_or_default(),
        }),
        _ => None,
    };

    tracing::info!(
        provider_id = %provider_id,
        account_id = %account.id,
        ok = outcome.ok,
        error_code = outcome.error_code.unwrap_or("none"),
        latency_ms = outcome.latency_ms,
        "provider connectivity test"
    );

    Ok(TestResult {
        ok: outcome.ok,
        latency_ms: outcome.latency_ms,
        checked_at: Utc::now(),
        account_id: Some(account.id),
        http_status: outcome.http_status,
        error,
    })
}

// ---------- 内部接口（供 routing/proxy，后续切片接入） ----------

#[allow(dead_code)]
pub async fn get_provider_for_proxy(
    state: &AppState,
    provider_id: Uuid,
) -> Result<Provider, AppError> {
    let pool = db(state)?;
    let row = repo::get_provider(pool, provider_id)
        .await?
        .ok_or_else(provider_not_found)?;
    Ok(to_provider(row))
}

#[allow(dead_code)]
pub async fn list_active_accounts(
    state: &AppState,
    provider_id: Uuid,
) -> Result<Vec<ProviderAccount>, AppError> {
    let pool = db(state)?;
    let rows = repo::list_accounts(pool, provider_id).await?;
    Ok(rows
        .into_iter()
        .filter(|row| row.status == "active")
        .map(to_account)
        .collect())
}

#[allow(dead_code)]
pub async fn decrypt_credential(
    state: &AppState,
    account_id: Uuid,
) -> Result<crypto::SecretString, AppError> {
    let pool = db(state)?;
    let (account, provider_active) = repo::account_for_decrypt(pool, account_id)
        .await?
        .ok_or_else(account_not_found)?;
    if !provider_active || account.status != "active" {
        return Err(AppError::not_found(
            "provider_disabled",
            "供应商或渠道账号已禁用，无法取用凭证",
        ));
    }
    crypto::open_credential(
        state.crypto.as_ref(),
        account.id,
        &account.credential_encrypted,
    )
}

// ---------- S4 proxy：账号选择（health / priority / weight / excluded） ----------

/// proxy 单次尝试选中的上游账号。
///
/// 凭证只在本次请求内存中使用：`SecretString` 的 Debug 已脱敏，本类型不实现 Serialize。
#[allow(dead_code)]
#[derive(Debug)]
pub struct SelectedAccount {
    pub account_id: Uuid,
    pub priority: i32,
    pub weight: i32,
    pub health: String,
    pub credential: crypto::SecretString,
}

/// `select_account` 的失败分类，proxy 按 proxy 技术设计 §5.1 映射为 503/502。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectAccountError {
    /// provider 不存在或 status != active。
    ProviderUnavailable,
    /// 无 active 账号，或全部被 excluded 排除。
    NoAvailableAccount,
    /// 凭证解密失败（主密钥问题）。
    CredentialUnavailable,
    /// 数据库不可用。
    DatabaseUnavailable,
}

/// health 优先级：healthy(0) > unknown(1) > unhealthy(2)；异常值按最差处理。
fn health_rank(health: &str) -> u8 {
    match health {
        "healthy" => 0,
        "unknown" => 1,
        _ => 2,
    }
}

/// 纯选择逻辑（无 IO，便于固定 RNG 单测）：
/// 1. 排除 `excluded`；
/// 2. 只在最佳 health 等级内选；
/// 3. 只取最小 priority；
/// 4. 同 priority 内按 weight 加权随机（Efraimidis–Spirakis `u^(1/w)` 取最大）；
///    `weight<=0` 不参与，整组权重全为 0 时按 1 处理；
/// 5. 平局按 `created_at ASC, id ASC` 兜底，保证可复现。
fn choose_candidate<'a>(
    rows: &'a [repo::SelectableAccountRow],
    excluded: &[Uuid],
    mut next_unit: impl FnMut() -> f64,
) -> Option<&'a repo::SelectableAccountRow> {
    let eligible: Vec<&repo::SelectableAccountRow> = rows
        .iter()
        .filter(|row| !excluded.contains(&row.id))
        .collect();
    let best_health = eligible.iter().map(|row| health_rank(&row.health)).min()?;
    let health_group: Vec<&repo::SelectableAccountRow> = eligible
        .into_iter()
        .filter(|row| health_rank(&row.health) == best_health)
        .collect();
    let best_priority = health_group.iter().map(|row| row.priority).min()?;
    let group: Vec<&repo::SelectableAccountRow> = health_group
        .into_iter()
        .filter(|row| row.priority == best_priority)
        .collect();

    let any_positive_weight = group.iter().any(|row| row.weight > 0);
    let mut best: Option<(&repo::SelectableAccountRow, f64)> = None;
    for row in group {
        let weight = if any_positive_weight {
            if row.weight <= 0 {
                continue;
            }
            row.weight as f64
        } else {
            1.0
        };
        let unit = next_unit();
        let key = unit.powf(1.0 / weight);
        let better = match best {
            None => true,
            Some((best_row, best_key)) => {
                key > best_key
                    || (key == best_key
                        && (row.created_at, row.id) < (best_row.created_at, best_row.id))
            }
        };
        if better {
            best = Some((row, key));
        }
    }
    best.map(|(row, _)| row)
}

/// proxy 单次尝试选一个账号并解密凭证（不写库，不更新 health/last_used）。
#[allow(dead_code)]
pub async fn select_account(
    state: &AppState,
    provider_id: Uuid,
    excluded: &[Uuid],
) -> Result<SelectedAccount, SelectAccountError> {
    let pool = state
        .db
        .as_ref()
        .ok_or(SelectAccountError::DatabaseUnavailable)?;
    let provider = repo::get_provider(pool, provider_id)
        .await
        .map_err(|_| SelectAccountError::DatabaseUnavailable)?
        .ok_or(SelectAccountError::ProviderUnavailable)?;
    if provider.status != "active" {
        return Err(SelectAccountError::ProviderUnavailable);
    }
    let rows = repo::selectable_accounts(pool, provider_id)
        .await
        .map_err(|_| SelectAccountError::DatabaseUnavailable)?;
    let chosen = choose_candidate(&rows, excluded, rand::random::<f64>)
        .ok_or(SelectAccountError::NoAvailableAccount)?;

    // 二次校验 + 取密文：账号/供应商可能在两步之间被禁用或删除。
    let (account, provider_active) = repo::account_for_decrypt(pool, chosen.id)
        .await
        .map_err(|_| SelectAccountError::DatabaseUnavailable)?
        .ok_or(SelectAccountError::NoAvailableAccount)?;
    if !provider_active {
        return Err(SelectAccountError::ProviderUnavailable);
    }
    if account.status != "active" {
        return Err(SelectAccountError::NoAvailableAccount);
    }
    let credential = crypto::open_credential(
        state.crypto.as_ref(),
        account.id,
        &account.credential_encrypted,
    )
    .map_err(|_| SelectAccountError::CredentialUnavailable)?;

    Ok(SelectedAccount {
        account_id: chosen.id,
        priority: chosen.priority,
        weight: chosen.weight,
        health: chosen.health.clone(),
        credential,
    })
}

/// routing 快照：上游模型 + provider 静态元数据（不含凭证/账号状态）。
#[allow(dead_code)]
pub async fn routing_upstream_views(
    state: &AppState,
) -> Result<Vec<crate::modules::providers::model::RoutingUpstreamView>, AppError> {
    let pool = db(state)?;
    let rows = repo::routing_upstream_rows(pool).await?;
    Ok(rows
        .into_iter()
        .map(
            |row| crate::modules::providers::model::RoutingUpstreamView {
                upstream_model_id: row.id,
                provider_id: row.provider_id,
                provider_name: row.provider_name,
                provider_kind: row.provider_kind,
                base_url: row.base_url,
                upstream_model: row.name,
                canonical: row.canonical,
                capabilities: serde_json::from_value(row.capabilities).unwrap_or_default(),
                context_window: row.context_window,
                max_output: row.max_output,
                price_input: row.price_input,
                price_output: row.price_output,
                currency: row.currency,
            },
        )
        .collect())
}

/// 管理台路由预览：实时 provider 状态 + active 账号数。
#[allow(dead_code)]
pub async fn provider_status_views(
    state: &AppState,
    provider_ids: &[Uuid],
) -> Result<Vec<crate::modules::providers::model::ProviderStatusView>, AppError> {
    let pool = db(state)?;
    let rows = repo::provider_status_rows(pool, provider_ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| crate::modules::providers::model::ProviderStatusView {
            provider_id: row.provider_id,
            status: row.status,
            active_account_count: row.active_account_count,
        })
        .collect())
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use chrono::TimeZone;

    fn candidate(
        id: u128,
        priority: i32,
        weight: i32,
        health: &str,
        created_second: u32,
    ) -> repo::SelectableAccountRow {
        repo::SelectableAccountRow {
            id: Uuid::from_u128(id),
            priority,
            weight,
            health: health.to_string(),
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, created_second)
                .single()
                .expect("valid timestamp"),
        }
    }

    /// 固定 RNG 序列；耗尽后回退 0.5，保证每个候选恰好消费一个值。
    fn fixed(values: &[f64]) -> impl FnMut() -> f64 + '_ {
        let mut values = values.iter().copied();
        move || values.next().unwrap_or(0.5)
    }

    #[test]
    fn excluded_accounts_are_skipped() {
        let rows = vec![
            candidate(1, 1, 100, "healthy", 0),
            candidate(2, 1, 100, "healthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[rows[0].id], fixed(&[]))
                .expect("second candidate")
                .id,
            rows[1].id
        );
        assert!(choose_candidate(&rows, &[rows[0].id, rows[1].id], fixed(&[])).is_none());
    }

    #[test]
    fn empty_candidates_return_none() {
        assert!(choose_candidate(&[], &[], fixed(&[])).is_none());
    }

    #[test]
    fn healthy_preferred_over_unknown_and_unhealthy() {
        // 故意让 unhealthy 的 priority/weight 更优：health 等级仍先决定。
        let rows = vec![
            candidate(1, 1, 100, "unhealthy", 0),
            candidate(2, 1, 100, "unknown", 1),
            candidate(3, 9, 1, "healthy", 2),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.99]))
                .expect("healthy candidate")
                .id,
            rows[2].id
        );
    }

    #[test]
    fn unknown_preferred_when_no_healthy() {
        let rows = vec![
            candidate(1, 1, 100, "unhealthy", 0),
            candidate(2, 2, 1, "unknown", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[]))
                .expect("unknown candidate")
                .id,
            rows[1].id
        );
    }

    #[test]
    fn unhealthy_used_only_as_last_resort() {
        let rows = vec![
            candidate(1, 1, 100, "unhealthy", 0),
            candidate(2, 3, 1, "unhealthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[]))
                .expect("unhealthy candidate")
                .id,
            rows[0].id
        );
    }

    #[test]
    fn lower_priority_wins_before_weight() {
        let rows = vec![
            candidate(1, 2, 100, "healthy", 0),
            candidate(2, 1, 1, "healthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.1]))
                .expect("priority candidate")
                .id,
            rows[1].id
        );
    }

    #[test]
    fn weight_zero_does_not_participate() {
        let rows = vec![
            candidate(1, 1, 0, "healthy", 0),
            candidate(2, 1, 100, "healthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[1.0, 0.0]))
                .expect("weighted candidate")
                .id,
            rows[1].id
        );
    }

    #[test]
    fn all_zero_weights_are_treated_as_one() {
        let rows = vec![
            candidate(1, 1, 0, "healthy", 0),
            candidate(2, 1, 0, "healthy", 1),
        ];
        // 权重全 0 时两者都参与，由 RNG 决定。
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.2, 0.8]))
                .expect("first or second")
                .id,
            rows[1].id
        );
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.9, 0.1]))
                .expect("first or second")
                .id,
            rows[0].id
        );
    }

    #[test]
    fn higher_weight_wins_with_same_rng_draw() {
        // Efraimidis key = u^(1/w)：相同 u 下权重越大 key 越大。
        let rows = vec![
            candidate(1, 1, 1, "healthy", 0),
            candidate(2, 1, 100, "healthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.5, 0.5]))
                .expect("weighted candidate")
                .id,
            rows[1].id
        );
    }

    #[test]
    fn tie_break_uses_created_at_then_id() {
        // 同权重、同 u → key 相等；created_at 更早者胜。
        let rows = vec![
            candidate(100, 1, 100, "healthy", 2),
            candidate(1, 1, 100, "healthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.5, 0.5]))
                .expect("earlier candidate")
                .id,
            rows[1].id
        );
        // created_at 相同 → id 更小者胜（与输入顺序无关）。
        let rows = vec![
            candidate(9, 1, 100, "healthy", 1),
            candidate(3, 1, 100, "healthy", 1),
        ];
        assert_eq!(
            choose_candidate(&rows, &[], fixed(&[0.5, 0.5]))
                .expect("smaller id")
                .id,
            rows[1].id
        );
    }

    #[test]
    fn selected_account_debug_redacts_credential() {
        let master = crate::platform::crypto::Crypto::from_key_bytes([7u8; 32]);
        let account_id = Uuid::new_v4();
        let sealed =
            crypto::seal_credential(&master, account_id, "sk-top-secret").expect("seal credential");
        let credential =
            crypto::open_credential(&master, account_id, &sealed).expect("open credential");
        let selected = SelectedAccount {
            account_id,
            priority: 1,
            weight: 100,
            health: "healthy".to_string(),
            credential,
        };
        let debug = format!("{selected:?}");
        assert!(
            !debug.contains("sk-top-secret"),
            "credential leaked: {debug}"
        );
        assert!(debug.contains("SecretString(***)"));
    }
}

#[cfg(test)]
mod db_flow_tests {
    use super::*;
    use crate::modules::providers::model::{
        CreateAccountRequest, CreateModelRequest, CreateProviderRequest, PatchAccountRequest,
        PatchProviderRequest,
    };
    use crate::platform::{config::Config, crypto::Crypto, state::AppState};
    use std::{sync::Arc, time::Instant};

    async fn test_state() -> Option<AppState> {
        let url = std::env::var("GW_TEST_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        let pool = sqlx::PgPool::connect(&url)
            .await
            .expect("connect GW_TEST_DATABASE_URL");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        sqlx::query("TRUNCATE providers CASCADE")
            .execute(&pool)
            .await
            .expect("truncate provider tables");
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("test bind addr"),
            database_url: Some(url),
            log_filter: "off".to_string(),
            run_migrations: false,
            cookie_secure: false,
            setup_token: None,
            usd_cny_rate: rust_decimal::Decimal::new(720, 2),
            trust_proxy_headers: false,
            proxy_max_body_bytes: 8 * 1024 * 1024,
            proxy_expose_provider_header: false,
            usage_retention_days: 180,
            rollup_retention_days: 1825,
            rollup_interval_secs: 300,
            request_log_retention_days: 30,
            audit_retention_days: 180,
            log_cleanup_interval_secs: 3600,
        };
        Some(AppState {
            config: Arc::new(config),
            db: Some(pool),
            crypto: Arc::new(Crypto::from_key_bytes([5u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        })
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE providers）"]
    async fn provider_crud_and_reference_rules() {
        let Some(state) = test_state().await else {
            return;
        };
        let pool = state.db.as_ref().expect("test pool");

        let (provider, account) = create_provider(
            &state,
            CreateProviderRequest {
                name: "TestProvider".to_string(),
                kind: "openai_compatible".to_string(),
                base_url: "http://127.0.0.1:9090/v1".to_string(),
                description: "test".to_string(),
                account: CreateAccountRequest {
                    name: "主账号".to_string(),
                    api_key: "sk-secret-1234".to_string(),
                    region: String::new(),
                    weight: None,
                    priority: None,
                    rpm_limit: None,
                    tpm_limit: None,
                },
            },
        )
        .await
        .expect("create provider");
        assert_eq!(provider.name, "TestProvider");
        assert_eq!(account.api_key_hint, "sk-…1234");

        // 密文非明文 + AAD 绑定
        let sealed: Vec<u8> =
            sqlx::query_scalar("SELECT credential_encrypted FROM provider_accounts WHERE id = $1")
                .bind(account.id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert!(!sealed.windows(9).any(|window| window == b"sk-secret"));
        let secret = crate::modules::providers::crypto::open_credential(
            state.crypto.as_ref(),
            account.id,
            &sealed,
        )
        .unwrap();
        assert_eq!(secret.expose(), "sk-secret-1234");

        // 重复名称 → 409
        let duplicate = create_provider(
            &state,
            CreateProviderRequest {
                name: "TestProvider".to_string(),
                kind: "openai_compatible".to_string(),
                base_url: "http://127.0.0.1:9090/v1".to_string(),
                description: String::new(),
                account: CreateAccountRequest {
                    name: "另一个".to_string(),
                    api_key: "sk-other".to_string(),
                    region: String::new(),
                    weight: None,
                    priority: None,
                    rpm_limit: None,
                    tpm_limit: None,
                },
            },
        )
        .await
        .expect_err("duplicate name must fail");
        assert!(matches!(
            duplicate,
            AppError::Conflict {
                code: "provider_name_conflict",
                ..
            }
        ));

        // 详情/列表
        let detail = get_provider(&state, provider.id).await.unwrap();
        assert_eq!(detail.account_count, 1);
        assert!(detail.routable);
        let page = list_providers(&state, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(page.total, 1);

        // ---------- S4 proxy：账号选择（select_account） ----------
        // 单账号：选中唯一 active 账号，凭证可解密，Debug 不泄露。
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("select single account");
        assert_eq!(selected.account_id, account.id);
        assert_eq!(selected.priority, account.priority);
        assert_eq!(selected.weight, account.weight);
        assert_eq!(selected.health, "unknown");
        assert_eq!(selected.credential.expose(), "sk-secret-1234");
        assert!(!format!("{selected:?}").contains("sk-secret-1234"));

        // 第二个账号 + excluded：排除本次已尝试/失败的账号。
        let backup = create_account(
            &state,
            provider.id,
            CreateAccountRequest {
                name: "备用账号".to_string(),
                api_key: "sk-second-9999".to_string(),
                region: String::new(),
                weight: Some(100),
                priority: Some(1),
                rpm_limit: None,
                tpm_limit: None,
            },
        )
        .await
        .expect("create backup account");
        let selected = select_account(&state, provider.id, &[account.id])
            .await
            .expect("select with excluded account");
        assert_eq!(selected.account_id, backup.id);
        assert_eq!(selected.credential.expose(), "sk-second-9999");
        assert!(matches!(
            select_account(&state, provider.id, &[account.id, backup.id]).await,
            Err(SelectAccountError::NoAvailableAccount)
        ));

        // 健康优先：healthy > unknown > unhealthy。
        sqlx::query("UPDATE provider_accounts SET health = 'unhealthy' WHERE id = $1")
            .bind(account.id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE provider_accounts SET health = 'healthy' WHERE id = $1")
            .bind(backup.id)
            .execute(pool)
            .await
            .unwrap();
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("select healthy account");
        assert_eq!(selected.account_id, backup.id);
        assert_eq!(selected.health, "healthy");
        // 没有 healthy 时退化到 unknown。
        sqlx::query("UPDATE provider_accounts SET health = 'unknown' WHERE id = $1")
            .bind(backup.id)
            .execute(pool)
            .await
            .unwrap();
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("select unknown account");
        assert_eq!(selected.account_id, backup.id);
        assert_eq!(selected.health, "unknown");
        // 前两类都不可用才用 unhealthy。
        sqlx::query("UPDATE provider_accounts SET health = 'unhealthy' WHERE id = $1")
            .bind(backup.id)
            .execute(pool)
            .await
            .unwrap();
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("select unhealthy account");
        assert_eq!(selected.health, "unhealthy");

        // priority 升序：priority 更小者优先（即便权重更低）。
        sqlx::query(
            "UPDATE provider_accounts SET health = 'unknown', priority = 2, weight = 100 \
             WHERE id = $1",
        )
        .bind(account.id)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE provider_accounts SET health = 'unknown', priority = 1, weight = 1 \
             WHERE id = $1",
        )
        .bind(backup.id)
        .execute(pool)
        .await
        .unwrap();
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("select by priority");
        assert_eq!(selected.account_id, backup.id);

        // weight=0 不参与；权重全 0 时仍可选中。
        sqlx::query("UPDATE provider_accounts SET priority = 1, weight = 0 WHERE id = $1")
            .bind(account.id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE provider_accounts SET priority = 1, weight = 100 WHERE id = $1")
            .bind(backup.id)
            .execute(pool)
            .await
            .unwrap();
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("select positive weight");
        assert_eq!(selected.account_id, backup.id);
        assert_eq!(selected.weight, 100);
        sqlx::query("UPDATE provider_accounts SET weight = 0 WHERE id = $1")
            .bind(backup.id)
            .execute(pool)
            .await
            .unwrap();
        let selected = select_account(&state, provider.id, &[])
            .await
            .expect("all zero weights selectable");
        assert!(selected.account_id == account.id || selected.account_id == backup.id);

        // 密文损坏 → CredentialUnavailable（不静默跳过账号）。
        sqlx::query("UPDATE provider_accounts SET credential_encrypted = $1 WHERE id = $2")
            .bind(vec![0u8, 1, 2, 3])
            .bind(account.id)
            .execute(pool)
            .await
            .unwrap();
        assert!(matches!(
            select_account(&state, provider.id, &[backup.id]).await,
            Err(SelectAccountError::CredentialUnavailable)
        ));
        // 恢复密文后可选。
        sqlx::query("UPDATE provider_accounts SET credential_encrypted = $1 WHERE id = $2")
            .bind(&sealed)
            .bind(account.id)
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(
            select_account(&state, provider.id, &[backup.id])
                .await
                .expect("restored credential")
                .credential
                .expose(),
            "sk-secret-1234"
        );

        // provider 禁用/启用
        let disabled = update_provider(
            &state,
            provider.id,
            PatchProviderRequest {
                name: None,
                base_url: None,
                description: None,
                status: Some("disabled".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(disabled.status, "disabled");
        // 禁用 provider → 该候选不可选（proxy 跳过候选）。
        assert!(matches!(
            select_account(&state, provider.id, &[]).await,
            Err(SelectAccountError::ProviderUnavailable)
        ));
        let detail = get_provider(&state, provider.id).await.unwrap();
        assert!(!detail.routable);
        update_provider(
            &state,
            provider.id,
            PatchProviderRequest {
                name: None,
                base_url: None,
                description: None,
                status: Some("active".to_string()),
            },
        )
        .await
        .unwrap();

        // 账号轮换 + 禁用
        let rotated = update_account(
            &state,
            provider.id,
            account.id,
            PatchAccountRequest {
                name: None,
                api_key: Some("sk-rotated-5678".to_string()),
                region: None,
                weight: None,
                priority: None,
                rpm_limit: None,
                tpm_limit: None,
                status: Some("disabled".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(rotated.api_key_hint, "sk-…5678");
        assert_eq!(rotated.status, "disabled");

        // 模型 + 引用规则
        let model = create_model(
            &state,
            provider.id,
            CreateModelRequest {
                name: "deepseek-chat".to_string(),
                canonical: "deepseek-chat".to_string(),
                capabilities: vec!["chat".to_string()],
                context_window: Some(65536),
                max_output: Some(8192),
                price_input: Some(0.5),
                price_output: Some(2.0),
                currency: Some("CNY".to_string()),
            },
        )
        .await
        .unwrap();
        sqlx::query(
            "WITH a AS (INSERT INTO model_aliases (id, alias) VALUES (gen_random_uuid(), 'test-alias') RETURNING id) \
             INSERT INTO model_alias_targets (id, alias_id, upstream_model_id) \
             SELECT gen_random_uuid(), a.id, $1 FROM a",
        )
        .bind(model.id)
        .execute(pool)
        .await
        .unwrap();

        let model_delete = delete_model(&state, provider.id, model.id)
            .await
            .expect_err("model in use must fail");
        assert!(matches!(
            model_delete,
            AppError::Conflict {
                code: "upstream_model_in_use",
                ..
            }
        ));
        let provider_delete = delete_provider(&state, provider.id)
            .await
            .expect_err("provider in use must fail");
        assert!(matches!(
            provider_delete,
            AppError::Conflict {
                code: "provider_in_use",
                ..
            }
        ));

        sqlx::query(
            "DELETE FROM model_aliases a USING model_alias_targets t \
             WHERE t.alias_id = a.id AND t.upstream_model_id = $1",
        )
        .bind(model.id)
        .execute(pool)
        .await
        .unwrap();
        delete_model(&state, provider.id, model.id).await.unwrap();
        delete_provider(&state, provider.id).await.unwrap();

        // 无 active 账号 / provider 不存在 → proxy 视为候选不可选。
        let (empty_provider, empty_account) = create_provider(
            &state,
            CreateProviderRequest {
                name: "EmptyProvider".to_string(),
                kind: "openai_compatible".to_string(),
                base_url: "http://127.0.0.1:9090/v1".to_string(),
                description: String::new(),
                account: CreateAccountRequest {
                    name: "唯一账号".to_string(),
                    api_key: "sk-empty-0000".to_string(),
                    region: String::new(),
                    weight: None,
                    priority: None,
                    rpm_limit: None,
                    tpm_limit: None,
                },
            },
        )
        .await
        .expect("create empty provider");
        assert_eq!(
            select_account(&state, empty_provider.id, &[])
                .await
                .expect("select before delete")
                .account_id,
            empty_account.id
        );
        delete_account(&state, empty_provider.id, empty_account.id)
            .await
            .expect("delete account");
        assert!(matches!(
            select_account(&state, empty_provider.id, &[]).await,
            Err(SelectAccountError::NoAvailableAccount)
        ));
        assert!(matches!(
            select_account(&state, Uuid::new_v4(), &[]).await,
            Err(SelectAccountError::ProviderUnavailable)
        ));
        delete_provider(&state, empty_provider.id).await.unwrap();

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM providers")
                .fetch_one(pool)
                .await
                .unwrap(),
            0
        );
    }
}
