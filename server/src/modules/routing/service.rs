//! routing 用例：别名 CRUD/预览、路由快照、`resolve_model`、`visible_models`。
//!
//! 解析顺序（catalog 迭代 3）：catalog active 组 → legacy alias 回退；
//! 只做配置选择；发布授权由 proxy 调 catalog、provider/账号健康由 proxy 实时校验。

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use rand::Rng;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::modules::catalog::{
    model::{RoutingDeploymentView, RoutingGroupView},
    service as catalog_service,
};
use crate::modules::providers::{
    model::{ProviderStatusView, RoutingUpstreamView},
    service as providers,
};
use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{
        CreateModelAliasRequest, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, MAX_PRIORITY, MAX_TARGETS,
        MAX_WEIGHT, MIN_PRIORITY, MIN_WEIGHT, ModelAliasAdmin, ModelAliasPreview, Page, PageQuery,
        PreviewCandidate, RouteCandidate, RouteError, RoutePlan, RouteSource, SNAPSHOT_TTL_SECS,
        TargetAdmin, TargetInput, TargetSpec, UpdateModelAliasRequest, VisibleModel,
    },
    repo,
};

pub fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

// ---------- 快照 ----------

#[derive(Debug, Clone)]
struct TargetSnapshot {
    upstream_model_id: Uuid,
    provider_id: Uuid,
    provider_name: String,
    provider_kind: String,
    base_url: String,
    upstream_model: String,
    canonical: String,
    capabilities: Vec<String>,
    context_window: Option<i32>,
    max_output: Option<i32>,
    price_input: Option<Decimal>,
    price_output: Option<Decimal>,
    currency: String,
    priority: i32,
    weight: i32,
    enabled: bool,
}

#[derive(Debug, Clone)]
struct AliasSnapshot {
    id: Uuid,
    alias: String,
    enabled: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    targets: Vec<TargetSnapshot>,
}

#[derive(Debug)]
struct Snapshot {
    version: u64,
    aliases: HashMap<String, AliasSnapshot>,
    built_at: Instant,
}

static CACHE: RwLock<Option<Arc<Snapshot>>> = RwLock::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);

/// 配置写后失效；测试/管理写路径调用。
pub fn invalidate() {
    if let Ok(mut guard) = CACHE.write() {
        *guard = None;
    }
    VERSION.fetch_add(1, Ordering::SeqCst);
}

async fn snapshot(state: &AppState) -> Result<Arc<Snapshot>, AppError> {
    let now = Instant::now();
    if let Ok(guard) = CACHE.read()
        && let Some(snapshot) = guard.as_ref()
        && now.duration_since(snapshot.built_at) < Duration::from_secs(SNAPSHOT_TTL_SECS)
    {
        return Ok(snapshot.clone());
    }

    let pool = db(state)?;
    let aliases = repo::all_aliases(pool).await?;
    let targets = repo::all_targets(pool).await?;
    let upstreams = providers::routing_upstream_views(state).await?;
    let upstream_map: HashMap<Uuid, RoutingUpstreamView> = upstreams
        .into_iter()
        .map(|view| (view.upstream_model_id, view))
        .collect();

    let mut grouped: HashMap<Uuid, Vec<TargetSnapshot>> = HashMap::new();
    for target in targets {
        let Some(view) = upstream_map.get(&target.upstream_model_id) else {
            continue;
        };
        grouped
            .entry(target.alias_id)
            .or_default()
            .push(TargetSnapshot {
                upstream_model_id: view.upstream_model_id,
                provider_id: view.provider_id,
                provider_name: view.provider_name.clone(),
                provider_kind: view.provider_kind.clone(),
                base_url: view.base_url.clone(),
                upstream_model: view.upstream_model.clone(),
                canonical: view.canonical.clone(),
                capabilities: view.capabilities.clone(),
                context_window: view.context_window,
                max_output: view.max_output,
                price_input: decimal_from_f64(view.price_input),
                price_output: decimal_from_f64(view.price_output),
                currency: view.currency.clone(),
                priority: target.priority,
                weight: target.weight,
                enabled: target.enabled,
            });
    }

    let mut alias_map = HashMap::new();
    for alias in aliases {
        let mut alias_targets = grouped.remove(&alias.id).unwrap_or_default();
        alias_targets.sort_by_key(|a| a.priority);
        alias_map.insert(
            alias.alias.clone(),
            AliasSnapshot {
                id: alias.id,
                alias: alias.alias,
                enabled: alias.enabled,
                created_at: alias.created_at,
                targets: alias_targets,
            },
        );
    }

    let version = VERSION.fetch_add(1, Ordering::SeqCst) + 1;
    let snapshot = Arc::new(Snapshot {
        version,
        aliases: alias_map,
        built_at: now,
    });
    if let Ok(mut guard) = CACHE.write() {
        *guard = Some(snapshot.clone());
    }
    Ok(snapshot)
}

fn decimal_from_f64(value: Option<f64>) -> Option<Decimal> {
    value.and_then(Decimal::from_f64_retain)
}

// ---------- 解析 ----------

/// 解析请求 model → 有序 RoutePlan；不调用上游、不校验 provider 实时状态。
///
/// 解析顺序（catalog 迭代 3）：
/// 1. catalog active 同名模型组有 ≥1 enabled 部署 → catalog 候选（不检查发布；发布授权由 proxy 前置）；
/// 2. active 组存在但无启用部署 → `NoRoute`（不回退 alias，避免静默换源）；
/// 3. 无 active 同名组（含 draft/archived）→ legacy alias 快照；
/// 4. 仍无 → `ModelNotFound`。
pub async fn resolve_model(state: &AppState, model: &str) -> Result<RoutePlan, RouteError> {
    let groups = catalog_service::routing_group_views(state)
        .await
        .map_err(|_| RouteError::DatabaseUnavailable)?;
    if let Some(group) = groups
        .iter()
        .find(|group| group.status == "active" && group.client_model_name == model)
    {
        let enabled: Vec<&RoutingDeploymentView> = group
            .deployments
            .iter()
            .filter(|deployment| deployment.enabled)
            .collect();
        if enabled.is_empty() {
            return Err(RouteError::NoRoute);
        }
        let version = catalog_service::catalog_version(state)
            .await
            .map_err(|_| RouteError::DatabaseUnavailable)?;
        let catalog_version = format!("cat:{version}");
        let candidates = order_catalog_candidates(group, &enabled);
        if candidates.is_empty() {
            return Err(RouteError::NoRoute);
        }
        return Ok(RoutePlan {
            requested_model: model.to_string(),
            source: RouteSource::Catalog,
            model_group_id: Some(group.group_id),
            alias_id: None,
            alias: group.client_model_name.clone(),
            catalog_version: Some(catalog_version),
            candidates,
        });
    }

    // 无 active catalog 组接管该名称：legacy alias 回退（draft/archived 不遮蔽）。
    let snapshot = snapshot(state)
        .await
        .map_err(|_| RouteError::DatabaseUnavailable)?;
    let Some(alias) = snapshot.aliases.get(model) else {
        return Err(RouteError::ModelNotFound);
    };
    if !alias.enabled {
        return Err(RouteError::ModelNotFound);
    }
    let mut candidates = order_candidates(alias);
    if candidates.is_empty() {
        return Err(RouteError::NoRoute);
    }
    Ok(RoutePlan {
        requested_model: model.to_string(),
        source: RouteSource::Alias,
        model_group_id: None,
        alias_id: Some(alias.id),
        alias: alias.alias.clone(),
        catalog_version: None,
        candidates: std::mem::take(&mut candidates),
    })
}

/// catalog 候选排序：priority 升序分组，组内按 weight 加权随机（同 alias 算法）。
fn order_catalog_candidates(
    group: &RoutingGroupView,
    enabled: &[&RoutingDeploymentView],
) -> Vec<RouteCandidate> {
    let mut groups: BTreeMap<i32, Vec<&RoutingDeploymentView>> = BTreeMap::new();
    for deployment in enabled {
        groups
            .entry(deployment.priority)
            .or_default()
            .push(deployment);
    }
    let mut rng = rand::thread_rng();
    let mut ordered = Vec::new();
    for (_, mut bucket) in groups {
        if bucket.len() > 1 {
            let mut keyed: Vec<(f64, &RoutingDeploymentView)> = bucket
                .drain(..)
                .map(|deployment| {
                    let weight = deployment.weight.max(1) as f64;
                    (rng.r#gen::<f64>().powf(1.0 / weight), deployment)
                })
                .collect();
            keyed.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            bucket = keyed
                .into_iter()
                .map(|(_, deployment)| deployment)
                .collect();
        }
        ordered.extend(
            bucket
                .into_iter()
                .map(|deployment| catalog_candidate(group, deployment)),
        );
    }
    ordered
}

fn catalog_candidate(
    group: &RoutingGroupView,
    deployment: &RoutingDeploymentView,
) -> RouteCandidate {
    RouteCandidate {
        upstream_model_id: deployment.upstream_model_id,
        provider_id: deployment.provider_id,
        provider_name: deployment.provider_name.clone(),
        provider_kind: deployment.provider_kind.clone(),
        base_url: deployment.base_url.clone(),
        upstream_model: deployment.upstream_model.clone(),
        canonical: deployment.canonical.clone(),
        priority: deployment.priority,
        weight: deployment.weight,
        capabilities: deployment.capabilities.clone(),
        context_window: deployment.context_window,
        max_output: deployment.max_output,
        price_input: deployment.price_input,
        price_output: deployment.price_output,
        currency: deployment.currency.clone(),
        deployment_id: Some(deployment.deployment_id),
        provider_account_id: Some(deployment.provider_account_id),
        model_group_id: Some(group.group_id),
    }
}

fn order_candidates(alias: &AliasSnapshot) -> Vec<RouteCandidate> {
    let mut groups: BTreeMap<i32, Vec<&TargetSnapshot>> = BTreeMap::new();
    for target in alias.targets.iter().filter(|target| target.enabled) {
        groups.entry(target.priority).or_default().push(target);
    }
    let mut rng = rand::thread_rng();
    let mut ordered = Vec::new();
    for (_, mut group) in groups {
        if group.len() > 1 {
            let mut keyed: Vec<(f64, &TargetSnapshot)> = group
                .drain(..)
                .map(|target| {
                    let weight = target.weight.max(1) as f64;
                    (rng.r#gen::<f64>().powf(1.0 / weight), target)
                })
                .collect();
            keyed.sort_by(|left, right| {
                right
                    .0
                    .partial_cmp(&left.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            group = keyed.into_iter().map(|(_, target)| target).collect();
        }
        ordered.extend(group.into_iter().map(candidate_from));
    }
    ordered
}

fn candidate_from(target: &TargetSnapshot) -> RouteCandidate {
    RouteCandidate {
        upstream_model_id: target.upstream_model_id,
        provider_id: target.provider_id,
        provider_name: target.provider_name.clone(),
        provider_kind: target.provider_kind.clone(),
        base_url: target.base_url.clone(),
        upstream_model: target.upstream_model.clone(),
        canonical: target.canonical.clone(),
        priority: target.priority,
        weight: target.weight,
        capabilities: target.capabilities.clone(),
        context_window: target.context_window,
        max_output: target.max_output,
        price_input: target.price_input,
        price_output: target.price_output,
        currency: target.currency.clone(),
        deployment_id: None,
        provider_account_id: None,
        model_group_id: None,
    }
}

/// `/v1/models` 投影：启用别名 + 白名单；不含价格/供应商/账号。
pub async fn visible_models(
    state: &AppState,
    allowed_models: &[String],
) -> Result<Vec<VisibleModel>, RouteError> {
    visible_models_with_version(state, allowed_models)
        .await
        .map(|(models, _)| models)
}

/// 与 [`visible_models`] 相同，但返回 routing 快照版本（`routing:<u64>`）。
///
/// catalog S4 `/v1/models` 用它在空列表时也能返回 `catalog_version`；
/// version 与列表来自同一快照，避免 TTL 边界不一致。
pub async fn visible_models_with_version(
    state: &AppState,
    allowed_models: &[String],
) -> Result<(Vec<VisibleModel>, u64), RouteError> {
    let snapshot = snapshot(state)
        .await
        .map_err(|_| RouteError::DatabaseUnavailable)?;
    let mut aliases: Vec<&AliasSnapshot> = snapshot
        .aliases
        .values()
        .filter(|alias| alias.enabled)
        .filter(|alias| {
            allowed_models.is_empty() || allowed_models.iter().any(|model| model == &alias.alias)
        })
        .filter(|alias| alias.targets.iter().any(|target| target.enabled))
        .collect();
    aliases.sort_by(|left, right| left.alias.cmp(&right.alias));
    let version = format!("routing:{}", snapshot.version);
    let models = aliases
        .into_iter()
        .filter_map(|alias| {
            let target = alias
                .targets
                .iter()
                .filter(|target| target.enabled)
                .min_by(|left, right| {
                    left.priority
                        .cmp(&right.priority)
                        .then(right.weight.cmp(&left.weight))
                        .then(left.upstream_model_id.cmp(&right.upstream_model_id))
                })?;
            Some(VisibleModel {
                id: alias.alias.clone(),
                object: "model".to_string(),
                created: alias.created_at,
                upstream_model: target.upstream_model.clone(),
                canonical: target.canonical.clone(),
                capabilities: target.capabilities.clone(),
                context_window: target.context_window,
                max_output: target.max_output,
                catalog_version: version.clone(),
            })
        })
        .collect();
    Ok((models, snapshot.version))
}

/// 轻量版本读取（空列表/健康检查场景）；返回 `routing:<u64>`。
pub async fn snapshot_version(state: &AppState) -> Result<String, RouteError> {
    let snapshot = snapshot(state)
        .await
        .map_err(|_| RouteError::DatabaseUnavailable)?;
    Ok(format!("routing:{}", snapshot.version))
}

// ---------- 管理台 ----------

fn alias_pattern(alias: &str) -> Result<String, AppError> {
    let alias = alias.trim();
    let valid = (1..=64).contains(&alias.len())
        && alias
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte));
    if !valid {
        return Err(AppError::validation_field(
            "alias",
            "模型别名必须 1–64 位，以字母/数字开头，仅含字母数字._:/-",
        ));
    }
    Ok(alias.to_string())
}

fn page_params(query: &PageQuery) -> Result<(i64, i64), AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(AppError::validation_field(
            "limit",
            format!("limit 必须在 1–{MAX_PAGE_LIMIT}"),
        ));
    }
    let offset = query.offset.unwrap_or(0);
    if offset < 0 {
        return Err(AppError::validation_field("offset", "offset 不能为负数"));
    }
    Ok((limit, offset))
}

fn validate_targets(
    targets: &[TargetInput],
    upstream_map: &HashMap<Uuid, RoutingUpstreamView>,
) -> Result<Vec<TargetSpec>, AppError> {
    if targets.is_empty() {
        return Err(AppError::validation_field("targets", "至少提供一个目标"));
    }
    if targets.len() > MAX_TARGETS {
        return Err(AppError::bad_request(
            "too_many_targets",
            format!("targets 最多 {MAX_TARGETS} 个"),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut specs = Vec::with_capacity(targets.len());
    for target in targets {
        if !seen.insert(target.upstream_model_id) {
            return Err(AppError::validation_field(
                "targets",
                "同一上游模型不能重复",
            ));
        }
        if !upstream_map.contains_key(&target.upstream_model_id) {
            return Err(AppError::not_found(
                "upstream_model_not_found",
                "上游模型不存在",
            ));
        }
        let priority = target.priority.unwrap_or(MIN_PRIORITY);
        if !(MIN_PRIORITY..=MAX_PRIORITY).contains(&priority) {
            return Err(AppError::validation_field(
                "targets",
                format!("priority 必须在 {MIN_PRIORITY}–{MAX_PRIORITY}"),
            ));
        }
        let weight = target.weight.unwrap_or(MAX_WEIGHT);
        if !(MIN_WEIGHT..=MAX_WEIGHT).contains(&weight) {
            return Err(AppError::validation_field(
                "targets",
                format!("weight 必须在 {MIN_WEIGHT}–{MAX_WEIGHT}"),
            ));
        }
        specs.push(TargetSpec {
            upstream_model_id: target.upstream_model_id,
            priority,
            weight,
            enabled: target.enabled.unwrap_or(true),
        });
    }
    if !specs.iter().any(|target| target.enabled) {
        return Err(AppError::bad_request(
            "no_enabled_targets",
            "至少需要一个启用的目标",
        ));
    }
    Ok(specs)
}

async fn upstream_views(state: &AppState) -> Result<Vec<RoutingUpstreamView>, AppError> {
    providers::routing_upstream_views(state).await
}

fn upstream_map(views: Vec<RoutingUpstreamView>) -> HashMap<Uuid, RoutingUpstreamView> {
    views
        .into_iter()
        .map(|view| (view.upstream_model_id, view))
        .collect()
}

async fn provider_statuses(
    state: &AppState,
    views: &[RoutingUpstreamView],
) -> Result<HashMap<Uuid, ProviderStatusView>, AppError> {
    let mut ids = views
        .iter()
        .map(|view| view.provider_id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    let statuses = providers::provider_status_views(state, &ids).await?;
    Ok(statuses
        .into_iter()
        .map(|status| (status.provider_id, status))
        .collect())
}

fn alias_admin(
    alias: repo::AliasRow,
    targets: &[repo::TargetRow],
    upstream_map: &HashMap<Uuid, RoutingUpstreamView>,
    statuses: &HashMap<Uuid, ProviderStatusView>,
) -> Result<ModelAliasAdmin, AppError> {
    let mut admin_targets = Vec::with_capacity(targets.len());
    for target in targets {
        let Some(upstream) = upstream_map.get(&target.upstream_model_id) else {
            return Err(AppError::internal("routing target upstream model missing"));
        };
        let status = statuses.get(&upstream.provider_id);
        admin_targets.push(TargetAdmin {
            id: target.id,
            upstream_model_id: target.upstream_model_id,
            upstream_model: upstream.upstream_model.clone(),
            canonical: upstream.canonical.clone(),
            provider_id: upstream.provider_id,
            provider_name: upstream.provider_name.clone(),
            provider_kind: upstream.provider_kind.clone(),
            provider_status: status
                .map(|status| status.status.clone())
                .unwrap_or_else(|| "active".to_string()),
            priority: target.priority,
            weight: target.weight,
            enabled: target.enabled,
            price_input: upstream.price_input,
            price_output: upstream.price_output,
            currency: upstream.currency.clone(),
            created_at: target.created_at,
            updated_at: target.updated_at,
        });
    }
    admin_targets.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(right.weight.cmp(&left.weight))
            .then(left.id.cmp(&right.id))
    });
    Ok(ModelAliasAdmin {
        id: alias.id,
        alias: alias.alias,
        enabled: alias.enabled,
        targets: admin_targets,
        created_at: alias.created_at,
        updated_at: alias.updated_at,
    })
}

async fn load_alias_admin(state: &AppState, id: Uuid) -> Result<ModelAliasAdmin, AppError> {
    let pool = db(state)?;
    let alias = repo::get_alias(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("model_alias_not_found", "模型别名不存在"))?;
    let targets = repo::list_targets(pool, id).await?;
    let views = upstream_views(state).await?;
    let map = upstream_map(views.clone());
    let statuses = provider_statuses(state, &views).await?;
    alias_admin(alias, &targets, &map, &statuses)
}

pub async fn list_model_aliases(
    state: &AppState,
    query: &PageQuery,
) -> Result<Page<ModelAliasAdmin>, AppError> {
    let pool = db(state)?;
    let (limit, offset) = page_params(query)?;
    let q = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    if q.is_some_and(|q| q.chars().count() > 64) {
        return Err(AppError::validation_field("q", "q 不能超过 64 个字符"));
    }
    let (aliases, total) = repo::list_aliases(pool, query.enabled, q, limit, offset).await?;
    let views = upstream_views(state).await?;
    let map = upstream_map(views.clone());
    let statuses = provider_statuses(state, &views).await?;
    let mut items = Vec::with_capacity(aliases.len());
    for alias in aliases {
        let targets = repo::list_targets(pool, alias.id).await?;
        items.push(alias_admin(alias, &targets, &map, &statuses)?);
    }
    Ok(Page {
        items,
        total,
        limit,
        offset,
    })
}

pub async fn get_model_alias(state: &AppState, id: Uuid) -> Result<ModelAliasAdmin, AppError> {
    load_alias_admin(state, id).await
}

pub async fn create_model_alias(
    state: &AppState,
    request: CreateModelAliasRequest,
) -> Result<ModelAliasAdmin, AppError> {
    let alias = alias_pattern(&request.alias)?;
    let views = upstream_views(state).await?;
    let map = upstream_map(views);
    let targets = validate_targets(&request.targets, &map)?;
    let pool = db(state)?;
    let id = Uuid::new_v4();
    repo::create_alias(pool, id, &alias, request.enabled.unwrap_or(true), &targets).await?;
    invalidate();
    load_alias_admin(state, id).await
}

pub async fn update_model_alias(
    state: &AppState,
    id: Uuid,
    request: UpdateModelAliasRequest,
) -> Result<ModelAliasAdmin, AppError> {
    if request.alias.is_none() && request.enabled.is_none() && request.targets.is_none() {
        return Err(AppError::validation_field("body", "至少提供一个字段"));
    }
    let pool = db(state)?;
    let current = repo::get_alias(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("model_alias_not_found", "模型别名不存在"))?;
    let alias = match request.alias {
        Some(raw) => alias_pattern(&raw)?,
        None => current.alias,
    };
    let enabled = request.enabled.unwrap_or(current.enabled);
    let targets = match request.targets {
        Some(inputs) => {
            let views = upstream_views(state).await?;
            Some(validate_targets(&inputs, &upstream_map(views))?)
        }
        None => None,
    };
    let updated = repo::update_alias(pool, id, &alias, enabled, targets.as_deref())
        .await?
        .ok_or_else(|| AppError::not_found("model_alias_not_found", "模型别名不存在"))?;
    drop(updated);
    invalidate();
    load_alias_admin(state, id).await
}

pub async fn delete_model_alias(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    repo::delete_alias(pool, id).await?;
    invalidate();
    Ok(())
}

pub async fn preview_model_alias(
    state: &AppState,
    id: Uuid,
) -> Result<ModelAliasPreview, AppError> {
    let pool = db(state)?;
    let alias = repo::get_alias(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("model_alias_not_found", "模型别名不存在"))?;
    let targets = repo::list_targets(pool, id).await?;
    let views = upstream_views(state).await?;
    let statuses = provider_statuses(state, &views).await?;
    let map = upstream_map(views);

    let mut warnings = Vec::new();
    if !alias.enabled {
        warnings.push("alias_disabled".to_string());
    }
    if !targets.iter().any(|target| target.enabled) {
        warnings.push("no_enabled_targets".to_string());
    }

    let mut candidates: Vec<PreviewCandidate> = targets
        .into_iter()
        .filter_map(|target| {
            let upstream = map.get(&target.upstream_model_id)?;
            let status = statuses.get(&upstream.provider_id);
            let mut target_warnings = Vec::new();
            if !target.enabled {
                target_warnings.push("target_disabled".to_string());
            }
            if status.is_some_and(|status| status.status != "active") {
                target_warnings.push("provider_disabled".to_string());
            }
            if status.is_some_and(|status| status.active_account_count == 0) {
                target_warnings.push("no_active_account".to_string());
            }
            Some(PreviewCandidate {
                upstream_model_id: target.upstream_model_id,
                upstream_model: upstream.upstream_model.clone(),
                provider_id: upstream.provider_id,
                provider_name: upstream.provider_name.clone(),
                provider_status: status
                    .map(|status| status.status.clone())
                    .unwrap_or_else(|| "active".to_string()),
                active_account_count: status
                    .map(|status| status.active_account_count)
                    .unwrap_or(0),
                priority: target.priority,
                weight: target.weight,
                enabled: target.enabled,
                warnings: target_warnings,
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(right.weight.cmp(&left.weight))
            .then(left.upstream_model_id.cmp(&right.upstream_model_id))
    });
    Ok(ModelAliasPreview {
        alias: alias.alias,
        enabled: alias.enabled,
        candidates,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(id: Uuid, provider_id: Uuid, name: &str) -> RoutingUpstreamView {
        RoutingUpstreamView {
            upstream_model_id: id,
            provider_id,
            provider_name: "P".to_string(),
            provider_kind: "openai_compatible".to_string(),
            base_url: "http://localhost/v1".to_string(),
            upstream_model: name.to_string(),
            canonical: name.to_string(),
            capabilities: vec!["chat".to_string()],
            context_window: Some(1024),
            max_output: Some(512),
            price_input: Some(0.1),
            price_output: Some(0.2),
            currency: "CNY".to_string(),
        }
    }

    #[test]
    fn alias_pattern_is_strict() {
        assert!(alias_pattern("fast").is_ok());
        assert!(alias_pattern("deepseek-chat-0731").is_ok());
        assert!(alias_pattern(" a ").is_ok());
        assert!(alias_pattern("").is_err());
        assert!(alias_pattern("-bad").is_err());
        assert!(alias_pattern("bad name").is_err());
        assert!(alias_pattern(&"a".repeat(65)).is_err());
    }

    #[test]
    fn target_validation_limits_and_duplicates() {
        let id = Uuid::new_v4();
        let map = HashMap::from([(id, view(id, Uuid::new_v4(), "m"))]);
        let good = validate_targets(
            &[TargetInput {
                upstream_model_id: id,
                priority: Some(1),
                weight: Some(50),
                enabled: Some(true),
            }],
            &map,
        )
        .unwrap();
        assert_eq!(good.len(), 1);
        assert!(
            validate_targets(
                &[TargetInput {
                    upstream_model_id: id,
                    priority: Some(1),
                    weight: Some(50),
                    enabled: Some(false),
                }],
                &map
            )
            .is_err()
        );
        assert!(
            validate_targets(
                &(0..21)
                    .map(|_| TargetInput {
                        upstream_model_id: id,
                        priority: Some(1),
                        weight: Some(50),
                        enabled: Some(true),
                    })
                    .collect::<Vec<_>>(),
                &map
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod db_flow_tests {
    use super::*;
    use crate::modules::routing::model::TargetInput;
    use crate::platform::{config::Config, crypto::Crypto};
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
        sqlx::query("TRUNCATE providers, model_aliases, model_groups CASCADE")
            .execute(&pool)
            .await
            .expect("truncate routing tables");
        invalidate();
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
            crypto: Arc::new(Crypto::from_key_bytes([7u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        })
    }

    async fn seed(state: &AppState) -> (Uuid, Uuid, Uuid, Uuid) {
        let pool = db(state).expect("pool");
        let provider_a = Uuid::new_v4();
        let provider_b = Uuid::new_v4();
        let model_a = Uuid::new_v4();
        let model_b = Uuid::new_v4();
        for (id, name) in [(provider_a, "ProviderA"), (provider_b, "ProviderB")] {
            sqlx::query(
                "INSERT INTO providers (id, name, kind, base_url) \
                 VALUES ($1, $2, 'openai_compatible', 'http://localhost/v1')",
            )
            .bind(id)
            .bind(name)
            .execute(pool)
            .await
            .expect("seed provider");
        }
        for (id, provider, name) in [
            (model_a, provider_a, "model-a"),
            (model_b, provider_b, "model-b"),
        ] {
            sqlx::query(
                "INSERT INTO upstream_models (id, provider_id, name, canonical, capabilities) \
                 VALUES ($1, $2, $3, $3, '[\"chat\"]'::jsonb)",
            )
            .bind(id)
            .bind(provider)
            .bind(name)
            .execute(pool)
            .await
            .expect("seed upstream model");
        }
        (provider_a, provider_b, model_a, model_b)
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE providers/model_aliases）"]
    async fn alias_crud_resolve_and_visible_models() {
        let Some(state) = test_state().await else {
            return;
        };
        let (provider_a, provider_b, model_a, model_b) = seed(&state).await;

        assert_eq!(
            list_model_aliases(&state, &PageQuery::default())
                .await
                .expect("list empty")
                .total,
            0
        );

        let created = create_model_alias(
            &state,
            CreateModelAliasRequest {
                alias: "fast".to_string(),
                enabled: Some(true),
                targets: vec![
                    TargetInput {
                        upstream_model_id: model_a,
                        priority: Some(1),
                        weight: Some(80),
                        enabled: Some(true),
                    },
                    TargetInput {
                        upstream_model_id: model_b,
                        priority: Some(2),
                        weight: Some(20),
                        enabled: Some(true),
                    },
                ],
            },
        )
        .await
        .expect("create alias");
        assert_eq!(created.alias, "fast");
        assert_eq!(created.targets.len(), 2);
        assert_eq!(created.targets[0].provider_id, provider_a);
        assert_eq!(created.targets[1].provider_id, provider_b);
        assert_eq!(created.targets[0].priority, 1);
        assert_eq!(created.targets[1].priority, 2);

        // 重复 alias
        assert!(matches!(
            create_model_alias(
                &state,
                CreateModelAliasRequest {
                    alias: "fast".to_string(),
                    enabled: None,
                    targets: vec![TargetInput {
                        upstream_model_id: model_a,
                        priority: None,
                        weight: None,
                        enabled: None,
                    }],
                }
            )
            .await,
            Err(AppError::Conflict {
                code: "model_alias_conflict",
                ..
            })
        ));

        // 目标不存在
        assert!(matches!(
            create_model_alias(
                &state,
                CreateModelAliasRequest {
                    alias: "missing-target".to_string(),
                    enabled: None,
                    targets: vec![TargetInput {
                        upstream_model_id: Uuid::new_v4(),
                        priority: None,
                        weight: None,
                        enabled: None,
                    }],
                }
            )
            .await,
            Err(AppError::NotFound {
                code: "upstream_model_not_found",
                ..
            })
        ));

        // 解析：主/备顺序
        let plan = resolve_model(&state, "fast").await.expect("resolve");
        assert_eq!(plan.alias, "fast");
        assert_eq!(plan.candidates.len(), 2);
        assert_eq!(plan.candidates[0].priority, 1);
        assert_eq!(plan.candidates[0].provider_id, provider_a);
        assert_eq!(plan.candidates[1].priority, 2);
        assert_eq!(plan.candidates[0].upstream_model, "model-a");

        // 预览确定性排序
        let preview = preview_model_alias(&state, created.id)
            .await
            .expect("preview");
        assert_eq!(preview.candidates[0].priority, 1);
        assert_eq!(preview.candidates[1].priority, 2);

        // 可见模型：白名单/停用
        let visible = visible_models(&state, &[]).await.expect("visible");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "fast");
        assert_eq!(visible[0].upstream_model, "model-a");
        assert!(
            visible_models(&state, &["other".to_string()])
                .await
                .expect("visible filtered")
                .is_empty()
        );

        // PATCH：改名/停用/替换 targets
        let updated = update_model_alias(
            &state,
            created.id,
            UpdateModelAliasRequest {
                alias: Some("fast-v2".to_string()),
                enabled: Some(true),
                targets: Some(vec![TargetInput {
                    upstream_model_id: model_b,
                    priority: Some(1),
                    weight: Some(100),
                    enabled: Some(true),
                }]),
            },
        )
        .await
        .expect("update alias");
        assert_eq!(updated.alias, "fast-v2");
        assert_eq!(updated.targets.len(), 1);
        assert!(matches!(
            resolve_model(&state, "fast").await,
            Err(RouteError::ModelNotFound)
        ));
        let plan = resolve_model(&state, "fast-v2")
            .await
            .expect("resolve updated");
        assert_eq!(plan.candidates[0].upstream_model, "model-b");

        // 停用/删除
        update_model_alias(
            &state,
            created.id,
            UpdateModelAliasRequest {
                alias: None,
                enabled: Some(false),
                targets: None,
            },
        )
        .await
        .expect("disable");
        assert!(matches!(
            resolve_model(&state, "fast-v2").await,
            Err(RouteError::ModelNotFound)
        ));
        delete_model_alias(&state, created.id)
            .await
            .expect("delete");
        assert_eq!(
            list_model_aliases(&state, &PageQuery::default())
                .await
                .expect("list after delete")
                .total,
            0
        );
    }

    async fn insert_account(pool: &sqlx::PgPool, provider_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO provider_accounts (id, provider_id, name, credential_encrypted)              VALUES ($1, $2, $3, '\\x01'::bytea)",
        )
        .bind(id)
        .bind(provider_id)
        .bind(format!("acct-{id}"))
        .execute(pool)
        .await
        .expect("insert account");
        id
    }

    async fn insert_group(pool: &sqlx::PgPool, name: &str, canonical: &str, status: &str) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO model_groups (id, client_model_name, canonical, status) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(name)
        .bind(canonical)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert model group");
        id
    }

    async fn insert_deployment(
        pool: &sqlx::PgPool,
        group_id: Uuid,
        upstream_model_id: Uuid,
        account_id: Uuid,
        priority: i32,
        enabled: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO deployments \
             (id, model_group_id, upstream_model_id, provider_account_id, priority, weight, enabled) \
             VALUES ($1, $2, $3, $4, $5, 100, $6)",
        )
        .bind(id)
        .bind(group_id)
        .bind(upstream_model_id)
        .bind(account_id)
        .bind(priority)
        .bind(enabled)
        .execute(pool)
        .await
        .expect("insert deployment");
        id
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE providers/model_aliases/model_groups）"]
    async fn catalog_first_resolution_and_alias_fallback() {
        let Some(state) = test_state().await else {
            return;
        };
        let (provider_a, provider_b, model_a, model_b) = seed(&state).await;
        let pool = db(&state).expect("pool").clone();
        let account_a = insert_account(&pool, provider_a).await;
        let account_b = insert_account(&pool, provider_b).await;

        // legacy alias 与 active catalog 组同名时，catalog 优先。
        create_model_alias(
            &state,
            CreateModelAliasRequest {
                alias: "draft-shadow".to_string(),
                enabled: Some(true),
                targets: vec![TargetInput {
                    upstream_model_id: model_a,
                    priority: Some(1),
                    weight: Some(100),
                    enabled: Some(true),
                }],
            },
        )
        .await
        .expect("legacy alias");

        // draft 组同名不遮蔽 alias。
        insert_group(&pool, "draft-shadow", "c1", "draft").await;
        let plan = resolve_model(&state, "draft-shadow")
            .await
            .expect("alias fallback");
        assert_eq!(plan.source, RouteSource::Alias);
        assert!(plan.model_group_id.is_none());
        assert!(plan.catalog_version.is_none());
        assert!(plan.alias_id.is_some());

        // active 组同名但 0 启用部署 → NoRoute，不 fallback alias。
        create_model_alias(
            &state,
            CreateModelAliasRequest {
                alias: "empty-shadow".to_string(),
                enabled: Some(true),
                targets: vec![TargetInput {
                    upstream_model_id: model_a,
                    priority: Some(1),
                    weight: Some(100),
                    enabled: Some(true),
                }],
            },
        )
        .await
        .expect("empty-shadow alias");
        insert_group(&pool, "empty-shadow", "c1", "active").await;
        assert!(matches!(
            resolve_model(&state, "empty-shadow").await,
            Err(RouteError::NoRoute)
        ));

        // 正常 catalog 组：候选按 priority 升序，带 deployment/account/版本。
        let group_id = insert_group(&pool, "group-model", "c1", "active").await;
        let deployment_low = insert_deployment(&pool, group_id, model_b, account_b, 2, true).await;
        let deployment_high = insert_deployment(&pool, group_id, model_a, account_a, 1, true).await;
        let plan = resolve_model(&state, "group-model")
            .await
            .expect("catalog plan");
        assert_eq!(plan.source, RouteSource::Catalog);
        assert_eq!(plan.model_group_id, Some(group_id));
        assert_eq!(plan.alias_id, None);
        assert!(
            plan.catalog_version
                .as_deref()
                .is_some_and(|version| version.starts_with("cat:"))
        );
        assert_eq!(plan.candidates.len(), 2);
        assert_eq!(plan.candidates[0].provider_id, provider_a);
        assert_eq!(plan.candidates[0].deployment_id, Some(deployment_high));
        assert_eq!(plan.candidates[0].provider_account_id, Some(account_a));
        assert_eq!(plan.candidates[0].model_group_id, Some(group_id));
        assert_eq!(plan.candidates[1].deployment_id, Some(deployment_low));
        assert_eq!(plan.candidates[1].priority, 2);

        // archived 组不遮蔽：删除 active 组后回退 alias。
        sqlx::query("UPDATE model_groups SET status = 'archived' WHERE id = $1")
            .bind(group_id)
            .execute(&pool)
            .await
            .expect("archive group");
        let plan = resolve_model(&state, "group-model").await;
        assert!(matches!(plan, Err(RouteError::ModelNotFound)));
    }
}
