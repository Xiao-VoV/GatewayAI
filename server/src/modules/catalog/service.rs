//! catalog service：S4 客户端投影 + v0.1.x 控制面（模型组 / 部署）。
//!
//! 边界：只读写 catalog 自有表（`model_groups`/`deployments`/`catalog_state`）；
//! providers/access 的只读数据经其 public service 获取，不读对方表。
//! 写事务：业务写 + `catalog_state.version + 1` 同事务（`repo::bump_version`）。

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
    time::Instant as StdInstant,
};

use sqlx::PgPool;
use uuid::Uuid;

use crate::modules::{
    access::{model::User, service as access_service},
    providers::{
        model::{Provider, ProviderAccount, ProviderStatusView, RoutingUpstreamView},
        service as providers_service,
    },
};
use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{
        self, AdminCatalogPreview, AdminPreviewModel, AdminPreviewToolDefault, CatalogError,
        CatalogPreviewQuery, CatalogSubject, ClientCatalogList, ClientModel, ClientModelDefault,
        ClientModelDefaultsList, ClientModelDetail, ConfirmMergeSuggestionRequest, DeploymentAdmin,
        DeploymentCreateRequest, DeploymentListQuery, DeploymentPatch, DeploymentSpec,
        DeploymentUpdateRequest, DiffItem, DiffValue, GroupFields, MergeConfirmAction,
        MergeConfirmation, MergeRejection, MergeSuggestion, ModelAuthorization, ModelGroupAdmin,
        ModelGroupCreateRequest, ModelGroupListQuery, ModelGroupRef, ModelGroupRoutePreview,
        ModelGroupSummary, ModelGroupUpdateRequest, Page, PreviewSubject, PublicationAdmin,
        PublicationCreateRequest, PublicationFields, PublicationListQuery, PublicationPatch,
        PublicationSummary, PublicationTargetAdmin, PublicationTargetSpec,
        PublicationUpdateRequest, RejectMergeSuggestionRequest, RelatedGroup,
        RoutePreviewCandidate, RoutingDeploymentView, RoutingGroupView, SuggestionDeployment,
        SuggestionListQuery, TargetView, ToolDefaultCreateRequest, ToolDefaultFields,
        ToolDefaultListQuery, ToolDefaultUpdateRequest, ToolDefaultView, ToolModelDefaultAdmin,
    },
    repo::{
        self, DeploymentFilters, DeploymentRow, GroupPublicationRow, MergeDecisionRow,
        ModelGroupRow, NewMergeDecision, PublicationRow, PublicationTargetRow, ToolDefaultModelRow,
        ToolDefaultRow,
    },
};

// ===========================================================================
// 控制面：公共
// ===========================================================================

pub fn db(state: &AppState) -> Result<&PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

// ---------- 进程内快照缓存（投影 / routing 视图） ----------

static PROJECTION_CACHE: OnceLock<crate::modules::catalog::cache::Cache<Projection>> =
    OnceLock::new();
static ROUTING_VIEW_CACHE: OnceLock<crate::modules::catalog::cache::Cache<Vec<RoutingGroupView>>> =
    OnceLock::new();

fn projection_cache() -> &'static crate::modules::catalog::cache::Cache<Projection> {
    PROJECTION_CACHE.get_or_init(|| {
        crate::modules::catalog::cache::Cache::new(crate::modules::catalog::cache::DEFAULT_TTL)
    })
}

fn routing_view_cache() -> &'static crate::modules::catalog::cache::Cache<Vec<RoutingGroupView>> {
    ROUTING_VIEW_CACHE.get_or_init(|| {
        crate::modules::catalog::cache::Cache::new(crate::modules::catalog::cache::DEFAULT_TTL)
    })
}

/// 写事务提交后失效投影与 routing 视图缓存（同实例下一次读立即可见）。
pub(crate) fn invalidate() {
    projection_cache().invalidate();
    routing_view_cache().invalidate();
}

/// 测试隔离：清空两处快照缓存（gated 直接 SQL 造数/跨用例），并恢复测试默认 TTL。
#[cfg(test)]
pub(crate) fn clear_caches() {
    projection_cache().clear();
    routing_view_cache().clear();
}

/// 测试专用：为缓存自测显式启用真实 TTL（默认测试 TTL=0 等价关闭缓存）。
#[cfg(test)]
pub(crate) fn set_snapshot_ttl_for_test(ttl: std::time::Duration) {
    projection_cache().set_ttl_for_test(ttl);
    routing_view_cache().set_ttl_for_test(ttl);
}

fn catalog_db_error(error: impl std::fmt::Display) -> CatalogError {
    tracing::warn!(error = %error, "catalog inference view unavailable");
    CatalogError::DatabaseUnavailable
}

fn decimal_from_f64(value: Option<f64>) -> Option<rust_decimal::Decimal> {
    value.and_then(rust_decimal::Decimal::from_f64_retain)
}

/// 当前目录版本（`catalog_state.version`）；与投影同快照（缓存命中时同一次读取）。
pub async fn catalog_version(state: &AppState) -> Result<u64, CatalogError> {
    let projection = load_projection(state).await.map_err(catalog_db_error)?;
    projection
        .version
        .strip_prefix("cat:")
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            tracing::error!(version = %projection.version, "catalog version format is invalid");
            CatalogError::DatabaseUnavailable
        })
}

/// routing 解析视图：全部状态模型组 + 部署 + providers 静态富化（上游模型/供应商/base_url/价格）。
///
/// - 进程内快照缓存（TTL 5s + 写后失效）；providers 静态元数据随视图缓存，
///   账号/健康由 proxy 实时校验（不进入本视图）；
/// - 不读发布/不选账号：发布授权由 proxy 调 [`authorize_model`]，账号由 proxy 校验/解密；
/// - 上游模型缺失的部署跳过并告警（配置异常不应让整条路由 503）。
pub async fn routing_group_views(state: &AppState) -> Result<Vec<RoutingGroupView>, CatalogError> {
    if let Some(cached) = routing_view_cache().get() {
        return Ok((*cached).clone());
    }
    let groups = load_routing_group_views_uncached(state)
        .await
        .map_err(catalog_db_error)?;
    routing_view_cache().store(Arc::new(groups.clone()));
    Ok(groups)
}

async fn load_routing_group_views_uncached(
    state: &AppState,
) -> Result<Vec<RoutingGroupView>, AppError> {
    let pool = db(state)?;
    let rows = repo::read_routing_state(pool).await?;
    let upstream_views = providers_service::routing_upstream_views(state).await?;
    let upstream: HashMap<Uuid, RoutingUpstreamView> = upstream_views
        .into_iter()
        .map(|view| (view.upstream_model_id, view))
        .collect();

    let mut deployments_by_group: HashMap<Uuid, Vec<RoutingDeploymentView>> = HashMap::new();
    for deployment in rows.deployments {
        let Some(group_id) = deployment.model_group_id else {
            continue;
        };
        let Some(upstream) = upstream.get(&deployment.upstream_model_id) else {
            tracing::warn!(
                deployment_id = %deployment.id,
                upstream_model_id = %deployment.upstream_model_id,
                "catalog deployment references a missing upstream model; skipped"
            );
            continue;
        };
        deployments_by_group
            .entry(group_id)
            .or_default()
            .push(RoutingDeploymentView {
                deployment_id: deployment.id,
                upstream_model_id: deployment.upstream_model_id,
                provider_account_id: deployment.provider_account_id,
                priority: deployment.priority,
                weight: deployment.weight,
                enabled: deployment.enabled,
                upstream_model: upstream.upstream_model.clone(),
                canonical: upstream.canonical.clone(),
                provider_id: upstream.provider_id,
                provider_name: upstream.provider_name.clone(),
                provider_kind: upstream.provider_kind.clone(),
                base_url: upstream.base_url.clone(),
                capabilities: upstream.capabilities.clone(),
                context_window: upstream.context_window,
                max_output: upstream.max_output,
                price_input: decimal_from_f64(upstream.price_input),
                price_output: decimal_from_f64(upstream.price_output),
                currency: upstream.currency.clone(),
            });
    }

    let groups = rows
        .groups
        .into_iter()
        .map(|group| RoutingGroupView {
            group_id: group.id,
            client_model_name: group.client_model_name,
            display_name: group.display_name,
            canonical: group.canonical,
            status: group.status,
            capabilities: model::string_array(&group.capabilities),
            context_window: group.context_window,
            max_output: group.max_output,
            deployments: deployments_by_group.remove(&group.id).unwrap_or_default(),
        })
        .collect();
    Ok(groups)
}

const PROVIDER_PAGE_LIMIT: i64 = 200;
/// 控制面只读富化的规模上限；超出视为配置异常（避免逐个 provider 拉账号的分页退化）。
const MAX_PROVIDERS: i64 = 1000;

/// providers 只读视图：上游模型/供应商静态信息 + 实时 provider 状态 + 账号列表。
struct ProviderViews {
    upstream: HashMap<Uuid, RoutingUpstreamView>,
    provider_status: HashMap<Uuid, String>,
    accounts: HashMap<Uuid, ProviderAccount>,
}

async fn load_all_providers(state: &AppState) -> Result<Vec<Provider>, AppError> {
    let mut items = Vec::new();
    let mut offset = 0_i64;
    loop {
        let page = providers_service::list_providers(
            state,
            None,
            None,
            Some(PROVIDER_PAGE_LIMIT),
            Some(offset),
        )
        .await?;
        if page.total > MAX_PROVIDERS {
            return Err(AppError::internal(
                "too many providers for catalog control-plane view",
            ));
        }
        let count = page.items.len() as i64;
        items.extend(page.items);
        offset += count;
        if count == 0 || offset >= page.total {
            break;
        }
    }
    Ok(items)
}

async fn load_provider_views(state: &AppState) -> Result<ProviderViews, AppError> {
    let upstream_views = providers_service::routing_upstream_views(state).await?;
    let upstream = upstream_views
        .iter()
        .cloned()
        .map(|view| (view.upstream_model_id, view))
        .collect();
    let provider_ids: Vec<Uuid> = upstream_views
        .iter()
        .map(|view| view.provider_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let statuses: Vec<ProviderStatusView> = if provider_ids.is_empty() {
        Vec::new()
    } else {
        providers_service::provider_status_views(state, &provider_ids).await?
    };
    let provider_status = statuses
        .into_iter()
        .map(|status| (status.provider_id, status.status))
        .collect();

    let mut accounts = HashMap::new();
    for provider in load_all_providers(state).await? {
        let list = providers_service::list_accounts(state, provider.id).await?;
        for account in list.items {
            accounts.insert(account.id, account);
        }
    }

    Ok(ProviderViews {
        upstream,
        provider_status,
        accounts,
    })
}

fn upstream_view(
    views: &ProviderViews,
    upstream_model_id: Uuid,
) -> Result<&RoutingUpstreamView, AppError> {
    views
        .upstream
        .get(&upstream_model_id)
        .ok_or_else(|| AppError::internal("deployment references a missing upstream model"))
}

fn provider_status_of(views: &ProviderViews, provider_id: Uuid) -> String {
    views
        .provider_status
        .get(&provider_id)
        .cloned()
        .unwrap_or_else(|| "disabled".to_string())
}

fn account_view(views: &ProviderViews, account_id: Uuid) -> Option<&ProviderAccount> {
    views.accounts.get(&account_id)
}

fn group_fields_from_row(row: &ModelGroupRow) -> GroupFields {
    GroupFields {
        client_model_name: row.client_model_name.clone(),
        display_name: row.display_name.clone(),
        canonical: row.canonical.clone(),
        description: row.description.clone(),
        capabilities: model::string_array(&row.capabilities),
        context_window: row.context_window,
        max_output: row.max_output,
        protocols: model::string_array(&row.protocols),
        parameters: row.parameters.clone(),
        price_input: row.price_input,
        price_output: row.price_output,
        currency: row.currency.clone(),
        status: row.status.clone(),
    }
}

fn sort_deployments(rows: &mut [DeploymentRow]) {
    rows.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(right.weight.cmp(&left.weight))
            .then(left.id.cmp(&right.id))
    });
}

fn build_deployment_admin(
    row: DeploymentRow,
    group_names: &HashMap<Uuid, String>,
    views: &ProviderViews,
) -> Result<DeploymentAdmin, AppError> {
    let upstream = upstream_view(views, row.upstream_model_id)?;
    let provider_status = provider_status_of(views, upstream.provider_id);
    let account = account_view(views, row.provider_account_id);
    let account_name = account
        .map(|account| account.name.clone())
        .unwrap_or_default();
    let account_region = account
        .map(|account| account.region.clone())
        .unwrap_or_default();
    let account_status = account
        .map(|account| account.status.clone())
        .unwrap_or_else(|| "disabled".to_string());
    let account_health = account
        .map(|account| account.health.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let price_missing = upstream.price_input.is_none() || upstream.price_output.is_none();
    let warnings = model::deployment_warnings(
        &provider_status,
        &account_status,
        &account_health,
        price_missing,
        row.model_group_id.is_none(),
    );
    Ok(DeploymentAdmin {
        id: row.id,
        model_group_id: row.model_group_id,
        model_group_client_name: row
            .model_group_id
            .and_then(|group_id| group_names.get(&group_id).cloned()),
        upstream_model_id: row.upstream_model_id,
        upstream_model: upstream.upstream_model.clone(),
        canonical: upstream.canonical.clone(),
        provider_id: upstream.provider_id,
        provider_name: upstream.provider_name.clone(),
        provider_kind: upstream.provider_kind.clone(),
        provider_status,
        provider_account_id: row.provider_account_id,
        account_name,
        account_region,
        account_status,
        account_health,
        priority: row.priority,
        weight: row.weight,
        enabled: row.enabled,
        price_input: upstream.price_input,
        price_output: upstream.price_output,
        currency: upstream.currency.clone(),
        context_window: upstream.context_window,
        max_output: upstream.max_output,
        notes: row.notes,
        warnings,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

async fn load_group_aggregations(
    pool: &PgPool,
    group_ids: &[Uuid],
) -> Result<(HashMap<Uuid, GroupPublicationRow>, HashMap<Uuid, i64>), AppError> {
    let publications = repo::list_group_publications(pool, group_ids).await?;
    let publication_map = publications
        .into_iter()
        .map(|publication| (publication.model_group_id, publication))
        .collect();
    let tool_default_counts = repo::count_tool_defaults_by_group(pool, group_ids).await?;
    Ok((publication_map, tool_default_counts))
}

fn build_group_summary(
    row: &ModelGroupRow,
    deployments: &[DeploymentRow],
    views: &ProviderViews,
    publication: Option<&GroupPublicationRow>,
    tool_default_count: i64,
) -> ModelGroupSummary {
    let enabled_deployment_count = deployments.iter().filter(|row| row.enabled).count() as i64;
    let any_price_missing = deployments.iter().any(|deployment| {
        views
            .upstream
            .get(&deployment.upstream_model_id)
            .is_none_or(|view| view.price_input.is_none() || view.price_output.is_none())
    });
    let warnings = model::group_warnings(&row.status, enabled_deployment_count, any_price_missing);
    ModelGroupSummary {
        id: row.id,
        client_model_name: row.client_model_name.clone(),
        display_name: row.display_name.clone(),
        canonical: row.canonical.clone(),
        status: row.status.clone(),
        capabilities: model::string_array(&row.capabilities),
        context_window: row.context_window,
        max_output: row.max_output,
        deployment_count: deployments.len() as i64,
        enabled_deployment_count,
        publication_status: publication.map(|publication| publication.status.clone()),
        tool_default_count,
        warnings,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn build_group_admin(
    row: ModelGroupRow,
    mut deployments: Vec<DeploymentRow>,
    views: &ProviderViews,
    publication: Option<&GroupPublicationRow>,
    tool_default_count: i64,
) -> Result<ModelGroupAdmin, AppError> {
    let summary = build_group_summary(&row, &deployments, views, publication, tool_default_count);
    let group_names = HashMap::from([(row.id, row.client_model_name.clone())]);
    sort_deployments(&mut deployments);
    let admin_deployments = deployments
        .into_iter()
        .map(|deployment| build_deployment_admin(deployment, &group_names, views))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ModelGroupAdmin {
        summary,
        description: row.description,
        protocols: model::string_array(&row.protocols),
        parameters: row.parameters,
        price_input: row.price_input,
        price_output: row.price_output,
        currency: row.currency,
        deployments: admin_deployments,
        publications: publication
            .map(|publication| PublicationSummary {
                id: publication.id,
                status: publication.status.clone(),
                target_count: publication.target_count,
                updated_at: publication.updated_at,
            })
            .into_iter()
            .collect(),
    })
}

fn build_preview_candidate(
    row: &DeploymentRow,
    views: &ProviderViews,
) -> Result<RoutePreviewCandidate, AppError> {
    let upstream = upstream_view(views, row.upstream_model_id)?;
    let provider_status = provider_status_of(views, upstream.provider_id);
    let account = account_view(views, row.provider_account_id);
    let account_name = account
        .map(|account| account.name.clone())
        .unwrap_or_default();
    let account_status = account
        .map(|account| account.status.clone())
        .unwrap_or_else(|| "disabled".to_string());
    let account_health = account
        .map(|account| account.health.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let price_missing = upstream.price_input.is_none() || upstream.price_output.is_none();
    let warnings = model::deployment_warnings(
        &provider_status,
        &account_status,
        &account_health,
        price_missing,
        false,
    );
    Ok(RoutePreviewCandidate {
        deployment_id: row.id,
        upstream_model_id: row.upstream_model_id,
        upstream_model: upstream.upstream_model.clone(),
        provider_id: upstream.provider_id,
        provider_name: upstream.provider_name.clone(),
        provider_status,
        provider_account_id: row.provider_account_id,
        account_name,
        account_status,
        account_health,
        priority: row.priority,
        weight: row.weight,
        enabled: row.enabled,
        warnings,
    })
}

/// 校验部署引用：上游模型/账号存在、同 provider、canonical 与组一致。
fn validate_deployment_specs(
    specs: &[DeploymentSpec],
    group_canonical: &str,
    views: &ProviderViews,
) -> Result<(), AppError> {
    for spec in specs {
        let upstream = views
            .upstream
            .get(&spec.upstream_model_id)
            .ok_or_else(|| AppError::not_found("upstream_model_not_found", "上游模型不存在"))?;
        let account = views
            .accounts
            .get(&spec.provider_account_id)
            .ok_or_else(|| AppError::not_found("provider_account_not_found", "渠道账号不存在"))?;
        if account.provider_id != upstream.provider_id {
            return Err(AppError::bad_request(
                "deployment_provider_mismatch",
                "上游模型与渠道账号必须属于同一供应商",
            ));
        }
        if upstream.canonical != group_canonical {
            return Err(AppError::bad_request(
                "canonical_mismatch",
                "部署上游模型的 canonical 与模型组不一致",
            ));
        }
    }
    Ok(())
}

fn group_not_found() -> AppError {
    AppError::not_found("model_group_not_found", "模型组不存在")
}

fn deployment_not_found() -> AppError {
    AppError::not_found("deployment_not_found", "部署不存在")
}

// ===========================================================================
// 控制面：模型组
// ===========================================================================

pub async fn list_model_groups(
    state: &AppState,
    query: ModelGroupListQuery,
) -> Result<Page<ModelGroupSummary>, AppError> {
    let page = model::validate_page(query.limit, query.offset)?;
    let status = model::validate_group_status_filter(query.status.as_deref())?;
    let (canonical, q) =
        model::validate_list_filters(query.canonical.as_deref(), query.q.as_deref())?;
    let pool = db(state)?;
    let (rows, total) = repo::list_model_groups(
        pool,
        status,
        canonical.as_deref(),
        q.as_deref(),
        page.limit,
        page.offset,
    )
    .await?;
    if rows.is_empty() {
        return Ok(Page {
            items: Vec::new(),
            total,
            limit: page.limit,
            offset: page.offset,
        });
    }
    let group_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let deployments = repo::list_deployments_for_groups(pool, &group_ids).await?;
    let views = load_provider_views(state).await?;
    let mut grouped: HashMap<Uuid, Vec<DeploymentRow>> = HashMap::new();
    for deployment in deployments {
        if let Some(group_id) = deployment.model_group_id {
            grouped.entry(group_id).or_default().push(deployment);
        }
    }
    let (publications, tool_default_counts) = load_group_aggregations(pool, &group_ids).await?;
    let items = rows
        .iter()
        .map(|row| {
            let empty = Vec::new();
            let deployments = grouped.get(&row.id).unwrap_or(&empty);
            build_group_summary(
                row,
                deployments,
                &views,
                publications.get(&row.id),
                tool_default_counts.get(&row.id).copied().unwrap_or(0),
            )
        })
        .collect();
    Ok(Page {
        items,
        total,
        limit: page.limit,
        offset: page.offset,
    })
}

pub async fn get_model_group(state: &AppState, id: Uuid) -> Result<ModelGroupAdmin, AppError> {
    let pool = db(state)?;
    let row = repo::get_model_group(pool, id)
        .await?
        .ok_or_else(group_not_found)?;
    let deployments = repo::list_deployments_for_group(pool, id).await?;
    let views = load_provider_views(state).await?;
    let (mut publications, tool_default_counts) = load_group_aggregations(pool, &[id]).await?;
    build_group_admin(
        row,
        deployments,
        &views,
        publications.remove(&id).as_ref(),
        tool_default_counts.get(&id).copied().unwrap_or(0),
    )
}

pub async fn create_model_group(
    state: &AppState,
    request: ModelGroupCreateRequest,
) -> Result<ModelGroupAdmin, AppError> {
    let (fields, specs) = model::validate_group_create(request)?;
    let views = load_provider_views(state).await?;
    validate_deployment_specs(&specs, &fields.canonical, &views)?;

    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let group_id = Uuid::new_v4();
    repo::insert_model_group(&mut tx, group_id, &fields).await?;
    for spec in &specs {
        repo::insert_deployment(&mut tx, Uuid::new_v4(), spec, Some(group_id)).await?;
    }
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    get_model_group(state, group_id).await
}

pub async fn update_model_group(
    state: &AppState,
    id: Uuid,
    request: ModelGroupUpdateRequest,
) -> Result<ModelGroupAdmin, AppError> {
    let patch = model::validate_group_update(request)?;
    let views = load_provider_views(state).await?;
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let Some(current) = repo::get_model_group_for_update(&mut tx, id).await? else {
        tx.rollback().await.ok();
        return Err(group_not_found());
    };
    let base = group_fields_from_row(&current);
    let next = model::apply_group_patch(&base, &patch);
    if let Err((field, message)) = model::check_context_pair(next.context_window, next.max_output) {
        tx.rollback().await.ok();
        return Err(AppError::validation_field(&field, message));
    }
    if next.canonical != base.canonical {
        let upstream_ids = repo::list_group_upstream_ids(&mut tx, id).await?;
        for upstream_id in upstream_ids {
            let upstream = views.upstream.get(&upstream_id).ok_or_else(|| {
                AppError::internal("deployment references a missing upstream model")
            })?;
            if upstream.canonical != next.canonical {
                tx.rollback().await.ok();
                return Err(AppError::bad_request(
                    "canonical_mismatch",
                    "已有部署的上游模型 canonical 与新的组 canonical 不一致",
                ));
            }
        }
    }
    if next.status == model::GROUP_STATUS_ACTIVE && current.status != model::GROUP_STATUS_ACTIVE {
        let (_, enabled) = repo::count_group_deployments(&mut tx, id).await?;
        if enabled == 0 {
            tx.rollback().await.ok();
            return Err(AppError::bad_request(
                "no_enabled_deployment",
                "active 模型组至少需要一个启用的部署",
            ));
        }
    }
    repo::update_model_group(&mut tx, id, &next)
        .await?
        .ok_or_else(group_not_found)?;
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    get_model_group(state, id).await
}

/// 归档（软删除）：已归档/不存在都返回成功（幂等）；只在真正变更时 bump 版本。
pub async fn archive_model_group(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let Some(_current) = repo::get_model_group_for_update(&mut tx, id).await? else {
        tx.rollback().await.ok();
        return Ok(());
    };
    if repo::archive_model_group(&mut tx, id).await? {
        repo::bump_version(&mut tx).await?;
    }
    tx.commit().await?;
    invalidate();
    Ok(())
}

pub async fn preview_model_group_routes(
    state: &AppState,
    id: Uuid,
) -> Result<ModelGroupRoutePreview, AppError> {
    let pool = db(state)?;
    let row = repo::get_model_group(pool, id)
        .await?
        .ok_or_else(group_not_found)?;
    let deployments = repo::list_deployments_for_group(pool, id).await?;
    let views = load_provider_views(state).await?;
    let mut candidates = deployments
        .iter()
        .map(|deployment| build_preview_candidate(deployment, &views))
        .collect::<Result<Vec<_>, _>>()?;
    candidates.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(right.weight.cmp(&left.weight))
            .then(left.deployment_id.cmp(&right.deployment_id))
    });
    let warnings = model::preview_warnings(&row.status, &candidates);
    Ok(ModelGroupRoutePreview {
        model_group_id: row.id,
        client_model_name: row.client_model_name,
        status: row.status,
        candidates,
        warnings,
    })
}

// ===========================================================================
// 控制面：部署
// ===========================================================================

pub async fn list_deployments(
    state: &AppState,
    query: DeploymentListQuery,
) -> Result<Page<DeploymentAdmin>, AppError> {
    let page = model::validate_page(query.limit, query.offset)?;
    let views = load_provider_views(state).await?;
    let provider_upstream_ids = query.provider_id.map(|provider_id| {
        views
            .upstream
            .values()
            .filter(|view| view.provider_id == provider_id)
            .map(|view| view.upstream_model_id)
            .collect::<Vec<_>>()
    });
    if provider_upstream_ids.as_ref().is_some_and(Vec::is_empty) {
        return Ok(Page {
            items: Vec::new(),
            total: 0,
            limit: page.limit,
            offset: page.offset,
        });
    }

    let pool = db(state)?;
    let filters = DeploymentFilters {
        model_group_id: query.model_group_id,
        upstream_model_id: query.upstream_model_id,
        provider_upstream_ids,
        provider_account_id: query.provider_account_id,
        enabled: query.enabled,
        ungrouped: query.ungrouped,
        limit: page.limit,
        offset: page.offset,
    };
    let (rows, total) = repo::list_deployments(pool, &filters).await?;
    if rows.is_empty() {
        return Ok(Page {
            items: Vec::new(),
            total,
            limit: page.limit,
            offset: page.offset,
        });
    }
    let group_ids: Vec<Uuid> = rows.iter().filter_map(|row| row.model_group_id).collect();
    let group_names = repo::list_group_client_names(pool, &group_ids).await?;
    let items = rows
        .into_iter()
        .map(|row| build_deployment_admin(row, &group_names, &views))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        items,
        total,
        limit: page.limit,
        offset: page.offset,
    })
}

pub async fn get_deployment(state: &AppState, id: Uuid) -> Result<DeploymentAdmin, AppError> {
    let pool = db(state)?;
    let row = repo::get_deployment(pool, id)
        .await?
        .ok_or_else(deployment_not_found)?;
    let mut group_names = HashMap::new();
    if let Some(group_id) = row.model_group_id
        && let Some(group) = repo::get_model_group(pool, group_id).await?
    {
        group_names.insert(group_id, group.client_model_name);
    }
    let views = load_provider_views(state).await?;
    build_deployment_admin(row, &group_names, &views)
}

pub async fn create_deployment(
    state: &AppState,
    request: DeploymentCreateRequest,
) -> Result<DeploymentAdmin, AppError> {
    let (spec, group_id) = model::validate_deployment_create(request)?;
    let views = load_provider_views(state).await?;
    let upstream = views
        .upstream
        .get(&spec.upstream_model_id)
        .ok_or_else(|| AppError::not_found("upstream_model_not_found", "上游模型不存在"))?;
    let account = views
        .accounts
        .get(&spec.provider_account_id)
        .ok_or_else(|| AppError::not_found("provider_account_not_found", "渠道账号不存在"))?;
    if account.provider_id != upstream.provider_id {
        return Err(AppError::bad_request(
            "deployment_provider_mismatch",
            "上游模型与渠道账号必须属于同一供应商",
        ));
    }
    let upstream_canonical = upstream.canonical.clone();

    let pool = db(state)?;
    if let Some(group_id) = group_id {
        validate_group_for_attach(pool, group_id, &upstream_canonical).await?;
    }
    let mut tx = pool.begin().await?;
    let deployment_id = Uuid::new_v4();
    repo::insert_deployment(&mut tx, deployment_id, &spec, group_id).await?;
    repo::bump_version(&mut tx).await?;
    if let Some(group_id) = group_id {
        repo::touch_model_group(&mut tx, group_id).await?;
    }
    tx.commit().await?;
    invalidate();
    get_deployment(state, deployment_id).await
}

pub async fn update_deployment(
    state: &AppState,
    id: Uuid,
    request: DeploymentUpdateRequest,
) -> Result<DeploymentAdmin, AppError> {
    let patch = model::validate_deployment_update(request)?;
    let views = load_provider_views(state).await?;
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let Some(current) = repo::get_deployment_for_update(&mut tx, id).await? else {
        tx.rollback().await.ok();
        return Err(deployment_not_found());
    };

    let spec = apply_deployment_patch(&current, &patch);
    let next_group_id = match patch.model_group_id {
        None => current.model_group_id,
        Some(group_id) => group_id,
    };
    if let Some(group_id) = next_group_id
        && Some(group_id) != current.model_group_id
    {
        let upstream = views
            .upstream
            .get(&spec.upstream_model_id)
            .ok_or_else(|| AppError::internal("deployment references a missing upstream model"))?;
        let group = repo::get_model_group_conn(&mut tx, group_id)
            .await?
            .ok_or_else(group_not_found)?;
        if group.status == model::GROUP_STATUS_ARCHIVED {
            tx.rollback().await.ok();
            return Err(AppError::bad_request(
                "model_group_not_active",
                "不能把部署加入已归档的模型组",
            ));
        }
        if group.canonical != upstream.canonical {
            tx.rollback().await.ok();
            return Err(AppError::bad_request(
                "canonical_mismatch",
                "部署上游模型的 canonical 与模型组不一致",
            ));
        }
    }

    repo::update_deployment(&mut tx, id, &spec, next_group_id)
        .await?
        .ok_or_else(deployment_not_found)?;
    repo::bump_version(&mut tx).await?;
    if let Some(group_id) = current.model_group_id {
        repo::touch_model_group(&mut tx, group_id).await?;
    }
    if let Some(group_id) = next_group_id
        && Some(group_id) != current.model_group_id
    {
        repo::touch_model_group(&mut tx, group_id).await?;
    }
    tx.commit().await?;
    invalidate();
    get_deployment(state, id).await
}

/// 物理删除部署（幂等）：不存在也返回成功；只在真正删除时 bump 版本。
pub async fn delete_deployment(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    if let Some(row) = repo::delete_deployment(&mut tx, id).await? {
        repo::bump_version(&mut tx).await?;
        if let Some(group_id) = row.model_group_id {
            repo::touch_model_group(&mut tx, group_id).await?;
        }
    }
    tx.commit().await?;
    invalidate();
    Ok(())
}

fn apply_deployment_patch(current: &DeploymentRow, patch: &DeploymentPatch) -> DeploymentSpec {
    DeploymentSpec {
        upstream_model_id: current.upstream_model_id,
        provider_account_id: current.provider_account_id,
        priority: patch.priority.unwrap_or(current.priority),
        weight: patch.weight.unwrap_or(current.weight),
        enabled: patch.enabled.unwrap_or(current.enabled),
        notes: patch.notes.clone().unwrap_or_else(|| current.notes.clone()),
    }
}

async fn validate_group_for_attach(
    pool: &PgPool,
    group_id: Uuid,
    upstream_canonical: &str,
) -> Result<(), AppError> {
    let group = repo::get_model_group(pool, group_id)
        .await?
        .ok_or_else(group_not_found)?;
    if group.status == model::GROUP_STATUS_ARCHIVED {
        return Err(AppError::bad_request(
            "model_group_not_active",
            "不能把部署加入已归档的模型组",
        ));
    }
    if group.canonical != upstream_canonical {
        return Err(AppError::bad_request(
            "canonical_mismatch",
            "部署上游模型的 canonical 与模型组不一致",
        ));
    }
    Ok(())
}

// ===========================================================================
// 发布管理（迭代 2）
// ===========================================================================

fn publication_not_found() -> AppError {
    AppError::not_found("publication_not_found", "发布不存在")
}

fn model_group_not_ready() -> AppError {
    AppError::bad_request(
        "model_group_not_ready",
        "模型组未就绪（需 active 且至少一个启用部署）",
    )
}

/// 校验发布对象引用的身份实体（user / user_group）存在；预留类型不做解析。
async fn validate_publication_target_refs(
    state: &AppState,
    targets: &[PublicationTargetSpec],
) -> Result<(), AppError> {
    let mut checked_users = std::collections::HashSet::new();
    let mut checked_groups = std::collections::HashSet::new();
    for target in targets {
        let Some(subject_id) = target.subject_id else {
            continue;
        };
        match target.subject_type.as_str() {
            "user" => {
                if !checked_users.insert(subject_id) {
                    continue;
                }
                if access_service::get_user_by_id(state, subject_id)
                    .await?
                    .is_none()
                {
                    return Err(AppError::not_found("user_not_found", "员工用户不存在"));
                }
            }
            "user_group" => {
                if !checked_groups.insert(subject_id) {
                    continue;
                }
                if !access_service::user_group_exists(state, subject_id).await? {
                    return Err(AppError::not_found("user_group_not_found", "用户组不存在"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn user_label(user: &User) -> String {
    if user.display_name.trim().is_empty() {
        user.email.clone()
    } else {
        user.display_name.clone()
    }
}

async fn publication_targets_admin(
    state: &AppState,
    targets: Vec<PublicationTargetRow>,
) -> Result<Vec<PublicationTargetAdmin>, AppError> {
    let mut users: HashMap<Uuid, Option<User>> = HashMap::new();
    let mut groups: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut out = Vec::with_capacity(targets.len());
    for target in targets {
        let subject_label = match target.subject_type.as_str() {
            "all_users" => Some("全部用户".to_string()),
            "user" => match target.subject_id {
                None => None,
                Some(user_id) => {
                    let user = match users.get(&user_id) {
                        Some(cached) => cached.clone(),
                        None => {
                            let fetched = access_service::get_user_by_id(state, user_id).await?;
                            users.insert(user_id, fetched.clone());
                            fetched
                        }
                    };
                    user.map(|user| user_label(&user))
                }
            },
            "user_group" => match target.subject_id {
                None => None,
                Some(group_id) => match groups.get(&group_id) {
                    Some(cached) => cached.clone(),
                    None => {
                        // 组被删除后 target 允许保留（多态无 FK），label 退化为 None（fail-closed）。
                        let label = if access_service::user_group_exists(state, group_id).await? {
                            Some(
                                access_service::get_user_group_detail(state, group_id)
                                    .await?
                                    .name,
                            )
                        } else {
                            None
                        };
                        groups.insert(group_id, label.clone());
                        label
                    }
                },
            },
            _ => None,
        };
        out.push(PublicationTargetAdmin {
            id: target.id,
            subject_type: target.subject_type,
            subject_id: target.subject_id,
            subject_label,
            visible: target.visible,
            allowed: target.allowed,
        });
    }
    Ok(out)
}

async fn build_publication_admin(
    state: &AppState,
    pool: &sqlx::PgPool,
    row: PublicationRow,
) -> Result<PublicationAdmin, AppError> {
    let targets = repo::list_targets_for_publications(pool, &[row.id]).await?;
    build_publication_admin_with_targets(state, row, targets).await
}

async fn build_publication_admin_with_targets(
    state: &AppState,
    row: PublicationRow,
    targets: Vec<PublicationTargetRow>,
) -> Result<PublicationAdmin, AppError> {
    let targets = publication_targets_admin(state, targets).await?;
    Ok(PublicationAdmin {
        id: row.id,
        model_group_id: row.model_group_id,
        model_group_client_name: row.model_group_client_name,
        status: row.status,
        data_level_max: row.data_level_max,
        quota_rpm: row.quota_rpm,
        quota_tpm: row.quota_tpm,
        quota_daily_requests: row.quota_daily_requests,
        note: row.note,
        created_by: row.created_by,
        targets,
        enforcement: model::Enforcement::default(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

async fn build_publication_admins(
    state: &AppState,
    pool: &sqlx::PgPool,
    rows: Vec<PublicationRow>,
) -> Result<Vec<PublicationAdmin>, AppError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let publication_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let targets = repo::list_targets_for_publications(pool, &publication_ids).await?;
    let mut by_publication: HashMap<Uuid, Vec<PublicationTargetRow>> = HashMap::new();
    for target in targets {
        by_publication
            .entry(target.publication_id)
            .or_default()
            .push(target);
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let targets = by_publication.remove(&row.id).unwrap_or_default();
        out.push(build_publication_admin_with_targets(state, row, targets).await?);
    }
    Ok(out)
}

pub async fn list_publications(
    state: &AppState,
    query: PublicationListQuery,
) -> Result<Page<PublicationAdmin>, AppError> {
    let page = model::validate_page(query.limit, query.offset)?;
    let (status, subject_type) = model::validate_publication_list_query(&query)?;
    let pool = db(state)?;
    let filters = repo::PublicationFilters {
        model_group_id: query.model_group_id,
        status,
        subject_type,
        subject_id: query.subject_id,
        limit: page.limit,
        offset: page.offset,
    };
    let (rows, total) = repo::list_publications(pool, &filters).await?;
    let items = build_publication_admins(state, pool, rows).await?;
    Ok(Page {
        items,
        total,
        limit: page.limit,
        offset: page.offset,
    })
}

pub async fn get_publication(state: &AppState, id: Uuid) -> Result<PublicationAdmin, AppError> {
    let pool = db(state)?;
    let row = repo::get_publication(pool, id)
        .await?
        .ok_or_else(publication_not_found)?;
    build_publication_admin(state, pool, row).await
}

pub async fn create_publication(
    state: &AppState,
    request: PublicationCreateRequest,
    created_by: Uuid,
) -> Result<PublicationAdmin, AppError> {
    let fields = model::validate_publication_create(request)?;
    let pool = db(state)?;
    let group = repo::get_model_group(pool, fields.model_group_id)
        .await?
        .ok_or_else(group_not_found)?;
    if group.status != model::GROUP_STATUS_ACTIVE {
        return Err(model_group_not_ready());
    }
    if repo::count_enabled_deployments(pool, group.id).await? == 0 {
        return Err(model_group_not_ready());
    }
    validate_publication_target_refs(state, &fields.targets).await?;

    let mut tx = pool.begin().await?;
    let publication_id = Uuid::new_v4();
    repo::insert_publication(&mut tx, publication_id, &fields, Some(created_by)).await?;
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    get_publication(state, publication_id).await
}

fn apply_publication_patch(
    current: &PublicationRow,
    patch: &PublicationPatch,
) -> PublicationFields {
    PublicationFields {
        model_group_id: current.model_group_id,
        status: patch
            .status
            .clone()
            .unwrap_or_else(|| current.status.clone()),
        data_level_max: patch.data_level_max.unwrap_or(current.data_level_max),
        quota_rpm: patch.quota_rpm.unwrap_or(current.quota_rpm),
        quota_tpm: patch.quota_tpm.unwrap_or(current.quota_tpm),
        quota_daily_requests: patch
            .quota_daily_requests
            .unwrap_or(current.quota_daily_requests),
        note: patch.note.clone().unwrap_or_else(|| current.note.clone()),
        targets: Vec::new(),
    }
}

pub async fn update_publication(
    state: &AppState,
    id: Uuid,
    request: PublicationUpdateRequest,
) -> Result<PublicationAdmin, AppError> {
    let patch = model::validate_publication_update(request)?;
    if let Some(targets) = &patch.targets {
        validate_publication_target_refs(state, targets).await?;
    }
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let Some(current) = repo::get_publication_for_update(&mut tx, id).await? else {
        tx.rollback().await.ok();
        return Err(publication_not_found());
    };
    let fields = apply_publication_patch(&current, &patch);
    repo::update_publication(&mut tx, id, &fields).await?;
    if let Some(targets) = &patch.targets {
        repo::replace_publication_targets(&mut tx, id, targets).await?;
    }
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    get_publication(state, id).await
}

/// 物理删除发布（幂等）：不存在也返回成功；只在真正删除时 bump 版本。
pub async fn delete_publication(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    if repo::delete_publication(&mut tx, id).await? {
        repo::bump_version(&mut tx).await?;
    }
    tx.commit().await?;
    invalidate();
    Ok(())
}

// ===========================================================================
// 客户端发布投影 + authorize_model（迭代 2）
// ===========================================================================

struct ProjectionDeployment {
    id: Uuid,
    upstream_model_id: Uuid,
    enabled: bool,
    priority: i32,
    weight: i32,
}

struct ProjectionGroup {
    /// 供 `group_by_id`/工具默认解析使用（客户端默认模型端点后续接线）。
    #[allow(dead_code)]
    id: Uuid,
    /// 该组当前 active 发布 id（无发布 = None；预览诊断字段）。
    #[allow(dead_code)]
    publication_id: Option<Uuid>,
    client_model_name: String,
    display_name: String,
    capabilities: Vec<String>,
    context_window: Option<i32>,
    max_output: Option<i32>,
    protocols: Vec<String>,
    parameters: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    deployments: Vec<ProjectionDeployment>,
    targets: Vec<TargetView>,
}

#[allow(dead_code)]
struct ProjectionToolDefault {
    id: Uuid,
    tool_key: String,
    scope_type: String,
    scope_id: Option<Uuid>,
    default_model_group_id: Uuid,
    allow_switch: bool,
    whitelist: Vec<Uuid>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

struct Projection {
    version: String,
    upstream: HashMap<Uuid, RoutingUpstreamView>,
    groups: Vec<ProjectionGroup>,
    #[allow(dead_code)]
    tool_defaults: Vec<ProjectionToolDefault>,
}

impl Projection {
    fn group(&self, client_model_name: &str) -> Option<&ProjectionGroup> {
        self.groups
            .iter()
            .find(|group| group.client_model_name == client_model_name)
    }

    #[allow(dead_code)]
    fn group_by_id(&self, group_id: Uuid) -> Option<&ProjectionGroup> {
        self.groups.iter().find(|group| group.id == group_id)
    }
}

/// 读取投影快照：命中进程内缓存直接返回；miss/TTL 过期时重建并缓存。
///
/// 版本与配置来自同一次 `REPEATABLE READ` 读取（`Projection.version` = `cat:<n>`）。
async fn load_projection(state: &AppState) -> Result<Arc<Projection>, AppError> {
    if let Some(cached) = projection_cache().get() {
        return Ok(cached);
    }
    let projection = Arc::new(load_projection_uncached(state).await?);
    projection_cache().store(projection.clone());
    Ok(projection)
}

async fn load_projection_uncached(state: &AppState) -> Result<Projection, AppError> {
    let pool = db(state)?;
    let rows = repo::read_projection_state(pool).await?;
    let upstream_views = providers_service::routing_upstream_views(state).await?;
    let upstream: HashMap<Uuid, RoutingUpstreamView> = upstream_views
        .into_iter()
        .map(|view| (view.upstream_model_id, view))
        .collect();

    let mut publication_by_group: HashMap<Uuid, Uuid> = HashMap::new();
    for publication in &rows.publications {
        publication_by_group.insert(publication.model_group_id, publication.id);
    }
    let mut deployment_by_group: HashMap<Uuid, Vec<ProjectionDeployment>> = HashMap::new();
    for deployment in &rows.deployments {
        if let Some(group_id) = deployment.model_group_id {
            deployment_by_group
                .entry(group_id)
                .or_default()
                .push(ProjectionDeployment {
                    id: deployment.id,
                    upstream_model_id: deployment.upstream_model_id,
                    enabled: deployment.enabled,
                    priority: deployment.priority,
                    weight: deployment.weight,
                });
        }
    }
    let mut targets_by_publication: HashMap<Uuid, Vec<TargetView>> = HashMap::new();
    for target in &rows.targets {
        targets_by_publication
            .entry(target.publication_id)
            .or_default()
            .push(TargetView {
                subject_type: target.subject_type.clone(),
                subject_id: target.subject_id,
                visible: target.visible,
                allowed: target.allowed,
            });
    }

    let mut whitelist_by_default: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for model in &rows.tool_default_models {
        whitelist_by_default
            .entry(model.tool_default_id)
            .or_default()
            .push(model.model_group_id);
    }
    let tool_defaults = rows
        .tool_defaults
        .iter()
        .map(|default| ProjectionToolDefault {
            id: default.id,
            tool_key: default.tool_key.clone(),
            scope_type: default.scope_type.clone(),
            scope_id: default.scope_id,
            default_model_group_id: default.default_model_group_id,
            allow_switch: default.allow_switch,
            whitelist: whitelist_by_default.remove(&default.id).unwrap_or_default(),
            updated_at: default.updated_at,
        })
        .collect();

    let groups = rows
        .groups
        .into_iter()
        .map(|group| {
            let publication_id = publication_by_group.get(&group.id).copied();
            let targets = publication_id
                .and_then(|publication_id| targets_by_publication.remove(&publication_id))
                .unwrap_or_default();
            ProjectionGroup {
                id: group.id,
                publication_id,
                client_model_name: group.client_model_name,
                display_name: group.display_name,
                capabilities: model::string_array(&group.capabilities),
                context_window: group.context_window,
                max_output: group.max_output,
                protocols: model::string_array(&group.protocols),
                parameters: group.parameters,
                created_at: group.created_at,
                deployments: deployment_by_group.remove(&group.id).unwrap_or_default(),
                targets,
            }
        })
        .collect();

    Ok(Projection {
        version: format!("cat:{}", rows.version),
        upstream,
        groups,
        tool_defaults,
    })
}

fn build_client_model(
    group: &ProjectionGroup,
    upstream: &HashMap<Uuid, RoutingUpstreamView>,
) -> ClientModel {
    let mut enabled: Vec<&ProjectionDeployment> = group
        .deployments
        .iter()
        .filter(|item| item.enabled)
        .collect();
    enabled.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(right.weight.cmp(&left.weight))
            .then(left.id.cmp(&right.id))
    });
    let upstream_model = enabled
        .first()
        .and_then(|deployment| upstream.get(&deployment.upstream_model_id))
        .map(|view| view.upstream_model.clone());
    let deployment_caps: Vec<Vec<String>> = enabled
        .iter()
        .filter_map(|deployment| upstream.get(&deployment.upstream_model_id))
        .map(|view| view.capabilities.clone())
        .collect();
    let capabilities = model::derive_capabilities(&group.capabilities, &deployment_caps);
    let context_values: Vec<Option<i32>> = enabled
        .iter()
        .map(|deployment| {
            upstream
                .get(&deployment.upstream_model_id)
                .and_then(|view| view.context_window)
        })
        .collect();
    let output_values: Vec<Option<i32>> = enabled
        .iter()
        .map(|deployment| {
            upstream
                .get(&deployment.upstream_model_id)
                .and_then(|view| view.max_output)
        })
        .collect();
    let context_window = model::derive_min_metric(group.context_window, &context_values);
    let max_output = model::derive_min_metric(group.max_output, &output_values);
    ClientModel {
        id: group.client_model_name.clone(),
        object: "model",
        created: group.created_at.timestamp(),
        owned_by: "gateway",
        display_name: if group.display_name.is_empty() {
            group.client_model_name.clone()
        } else {
            group.display_name.clone()
        },
        upstream_model,
        capabilities,
        context_window,
        max_output,
        protocols: group.protocols.clone(),
    }
}

/// 客户端目录（发布投影）：只返回 active 组 + 生效发布可见 + Key 白名单允许的模型。
pub async fn client_catalog(
    state: &AppState,
    subject: &CatalogSubject,
) -> Result<ClientCatalogList, AppError> {
    let projection = load_projection(state).await?;
    let mut data = Vec::new();
    for group in &projection.groups {
        let Some(effective) =
            model::effective_publication(&group.targets, subject, &group.client_model_name)
        else {
            continue;
        };
        if !effective.visible {
            continue;
        }
        data.push(build_client_model(group, &projection.upstream));
    }
    Ok(ClientCatalogList {
        object: "list",
        data,
        catalog_version: projection.version.clone(),
    })
}

/// 客户端模型详情：不可见/不存在统一返回 `None`（handler → 404 `model_not_found`）。
pub async fn client_model_detail(
    state: &AppState,
    subject: &CatalogSubject,
    client_model_name: &str,
) -> Result<Option<ClientModelDetail>, AppError> {
    let projection = load_projection(state).await?;
    let Some(group) = projection.group(client_model_name) else {
        return Ok(None);
    };
    let Some(effective) = model::effective_publication(&group.targets, subject, client_model_name)
    else {
        return Ok(None);
    };
    if !effective.visible {
        return Ok(None);
    }
    Ok(Some(ClientModelDetail {
        model: build_client_model(group, &projection.upstream),
        parameters: group.parameters.clone(),
        catalog_version: projection.version.clone(),
    }))
}

/// 发布授权判定（proxy 推理入口调用；不涉及 Key 白名单之外的其他策略）。
///
/// 注：proxy 尚未接入，本迭代先固化签名与语义（后续 iteration 由 proxy 调用）。
#[allow(dead_code)]
pub async fn authorize_model(
    state: &AppState,
    subject: &CatalogSubject,
    model_name: &str,
) -> Result<ModelAuthorization, CatalogError> {
    let projection = match load_projection(state).await {
        Ok(projection) => projection,
        Err(error) => {
            tracing::warn!(error = %error, "catalog authorize_model projection failed");
            return Err(CatalogError::DatabaseUnavailable);
        }
    };
    let Some(group) = projection.group(model_name) else {
        return Ok(ModelAuthorization::Unknown);
    };
    match model::effective_publication(&group.targets, subject, model_name) {
        None => Ok(ModelAuthorization::NotPublished),
        Some(effective) if !effective.visible => Ok(ModelAuthorization::NotPublished),
        Some(effective) if !effective.allowed => Ok(ModelAuthorization::VisibleOnly),
        Some(_) => Ok(ModelAuthorization::Allowed),
    }
}

// ===========================================================================
// 工具默认模型（迭代 4）
// ===========================================================================

fn tool_default_not_found() -> AppError {
    AppError::not_found("tool_model_default_not_found", "工具默认模型不存在")
}

fn user_not_found() -> AppError {
    AppError::not_found("user_not_found", "员工用户不存在")
}

async fn validate_tool_default_refs(
    state: &AppState,
    fields: &ToolDefaultFields,
) -> Result<(), AppError> {
    let pool = db(state)?;
    let default_group = repo::get_model_group(pool, fields.default_model_group_id)
        .await?
        .ok_or_else(group_not_found)?;
    if default_group.status != model::GROUP_STATUS_ACTIVE {
        return Err(AppError::bad_request(
            "model_group_not_active",
            "默认模型组未激活（active），不能设为工具默认模型",
        ));
    }
    for group_id in &fields.allowed_model_group_ids {
        let group = repo::get_model_group(pool, *group_id)
            .await?
            .ok_or_else(group_not_found)?;
        if group.status != model::GROUP_STATUS_ACTIVE {
            return Err(AppError::bad_request(
                "model_group_not_active",
                "可切换白名单中的模型组未激活（active）",
            ));
        }
    }
    match fields.scope_type.as_str() {
        "user" => {
            if let Some(user_id) = fields.scope_id
                && access_service::get_user_by_id(state, user_id)
                    .await?
                    .is_none()
            {
                return Err(user_not_found());
            }
        }
        "user_group" => {
            if let Some(group_id) = fields.scope_id
                && !access_service::user_group_exists(state, group_id).await?
            {
                return Err(AppError::not_found("user_group_not_found", "用户组不存在"));
            }
        }
        _ => {}
    }
    Ok(())
}

async fn tool_default_scope_label(
    state: &AppState,
    row: &ToolDefaultRow,
) -> Result<Option<String>, AppError> {
    match row.scope_type.as_str() {
        "global" => Ok(Some("全部用户".to_string())),
        "user" => match row.scope_id {
            Some(user_id) => Ok(access_service::get_user_by_id(state, user_id)
                .await?
                .map(|user| user_label(&user))),
            None => Ok(None),
        },
        "user_group" => match row.scope_id {
            Some(group_id) => {
                if access_service::user_group_exists(state, group_id).await? {
                    Ok(Some(
                        access_service::get_user_group_detail(state, group_id)
                            .await?
                            .name,
                    ))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        },
        _ => Ok(None),
    }
}

async fn build_tool_default_admin_with_refs(
    state: &AppState,
    pool: &PgPool,
    row: ToolDefaultRow,
    refs: Vec<ToolDefaultModelRow>,
) -> Result<ToolModelDefaultAdmin, AppError> {
    let group_ids: Vec<Uuid> = refs.iter().map(|item| item.model_group_id).collect();
    let names = repo::list_group_client_names(pool, &group_ids).await?;
    let allowed_model_groups = refs
        .iter()
        .filter_map(|item| {
            names.get(&item.model_group_id).map(|name| ModelGroupRef {
                id: item.model_group_id,
                client_model_name: name.clone(),
            })
        })
        .collect();
    Ok(ToolModelDefaultAdmin {
        id: row.id,
        tool_key: row.tool_key.clone(),
        scope_type: row.scope_type.clone(),
        scope_id: row.scope_id,
        scope_label: tool_default_scope_label(state, &row).await?,
        default_model_group_id: row.default_model_group_id,
        default_model_client_name: row.default_model_client_name,
        allow_switch: row.allow_switch,
        allowed_model_groups,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

async fn build_tool_default_admins(
    state: &AppState,
    pool: &PgPool,
    rows: Vec<ToolDefaultRow>,
) -> Result<Vec<ToolModelDefaultAdmin>, AppError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let refs = repo::list_tool_default_models_for_defaults(pool, &ids).await?;
    let mut refs_by_default: HashMap<Uuid, Vec<ToolDefaultModelRow>> = HashMap::new();
    for item in refs {
        refs_by_default
            .entry(item.tool_default_id)
            .or_default()
            .push(item);
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let refs = refs_by_default.remove(&row.id).unwrap_or_default();
        out.push(build_tool_default_admin_with_refs(state, pool, row, refs).await?);
    }
    Ok(out)
}

pub async fn list_tool_model_defaults(
    state: &AppState,
    query: ToolDefaultListQuery,
) -> Result<Page<ToolModelDefaultAdmin>, AppError> {
    let page = model::validate_page(query.limit, query.offset)?;
    let (tool_key, scope_type) = model::validate_tool_default_list_query(&query)?;
    let pool = db(state)?;
    let filters = repo::ToolDefaultFilters {
        tool_key,
        scope_type,
        scope_id: query.scope_id,
        limit: page.limit,
        offset: page.offset,
    };
    let (rows, total) = repo::list_tool_model_defaults(pool, &filters).await?;
    let items = build_tool_default_admins(state, pool, rows).await?;
    Ok(Page {
        items,
        total,
        limit: page.limit,
        offset: page.offset,
    })
}

pub async fn get_tool_model_default(
    state: &AppState,
    id: Uuid,
) -> Result<ToolModelDefaultAdmin, AppError> {
    let pool = db(state)?;
    let row = repo::get_tool_model_default(pool, id)
        .await?
        .ok_or_else(tool_default_not_found)?;
    let refs = repo::list_tool_default_models_for_defaults(pool, &[row.id]).await?;
    build_tool_default_admin_with_refs(state, pool, row, refs).await
}

pub async fn create_tool_model_default(
    state: &AppState,
    request: ToolDefaultCreateRequest,
) -> Result<ToolModelDefaultAdmin, AppError> {
    let fields = model::validate_tool_default_create(request)?;
    validate_tool_default_refs(state, &fields).await?;
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let id = Uuid::new_v4();
    repo::insert_tool_default(&mut tx, id, &fields).await?;
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    get_tool_model_default(state, id).await
}

pub async fn update_tool_model_default(
    state: &AppState,
    id: Uuid,
    request: ToolDefaultUpdateRequest,
) -> Result<ToolModelDefaultAdmin, AppError> {
    let patch = model::validate_tool_default_update(request)?;
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    let Some(current) = repo::get_tool_model_default_for_update(&mut tx, id).await? else {
        tx.rollback().await.ok();
        return Err(tool_default_not_found());
    };
    let current_whitelist = repo::list_tool_default_models_conn(&mut tx, id).await?;
    let fields = ToolDefaultFields {
        tool_key: current.tool_key.clone(),
        scope_type: current.scope_type.clone(),
        scope_id: current.scope_id,
        default_model_group_id: patch
            .default_model_group_id
            .unwrap_or(current.default_model_group_id),
        allow_switch: patch.allow_switch.unwrap_or(current.allow_switch),
        allowed_model_group_ids: match &patch.allowed_model_group_ids {
            Some(values) => values.clone(),
            None => current_whitelist,
        },
    };
    validate_tool_default_refs(state, &fields).await?;
    repo::update_tool_default(&mut tx, id, &fields).await?;
    if patch.allowed_model_group_ids.is_some() {
        repo::replace_tool_default_models(&mut tx, id, &fields.allowed_model_group_ids).await?;
    }
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    get_tool_model_default(state, id).await
}

/// 物理删除工具默认（幂等）：不存在也成功；只在真正删除时 bump 版本。
pub async fn delete_tool_model_default(state: &AppState, id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    let mut tx = pool.begin().await?;
    if repo::delete_tool_default(&mut tx, id).await? {
        repo::bump_version(&mut tx).await?;
    }
    tx.commit().await?;
    invalidate();
    Ok(())
}

#[allow(dead_code)]
fn tool_scope_rank(scope_type: &str) -> u8 {
    match scope_type {
        "user" => 3,
        "user_group" => 2,
        _ => 1,
    }
}

#[allow(dead_code)]
fn build_tool_default_view(
    default: &ProjectionToolDefault,
    projection: &Projection,
    subject: &CatalogSubject,
) -> ToolDefaultView {
    let usable_group = |group_id: Uuid| -> Option<&ProjectionGroup> {
        let group = projection.group_by_id(group_id)?;
        match model::effective_publication(&group.targets, subject, &group.client_model_name) {
            Some(effective) if effective.visible && effective.allowed => Some(group),
            _ => None,
        }
    };

    let mut warnings: Vec<String> = Vec::new();
    let effective_default = usable_group(default.default_model_group_id);
    if effective_default.is_none() {
        match projection.group_by_id(default.default_model_group_id) {
            None => warnings.push("default_model_group_inactive".to_string()),
            Some(group) => {
                match model::effective_publication(
                    &group.targets,
                    subject,
                    &group.client_model_name,
                ) {
                    None => warnings.push("default_model_not_visible".to_string()),
                    Some(effective) if !effective.visible => {
                        warnings.push("default_model_not_visible".to_string());
                    }
                    Some(_) => warnings.push("default_model_not_allowed".to_string()),
                }
            }
        }
    }

    let mut switchable_model_groups: Vec<ModelGroupRef> = Vec::new();
    if default.allow_switch {
        let mut candidate_ids: Vec<Uuid> = Vec::new();
        if let Some(group) = effective_default {
            candidate_ids.push(group.id);
        }
        for group_id in &default.whitelist {
            if !candidate_ids.contains(group_id) {
                candidate_ids.push(*group_id);
            }
        }
        let mut filtered = 0_usize;
        for group_id in candidate_ids {
            match usable_group(group_id) {
                Some(group) => switchable_model_groups.push(ModelGroupRef {
                    id: group.id,
                    client_model_name: group.client_model_name.clone(),
                }),
                None => filtered += 1,
            }
        }
        if filtered > 0 {
            warnings.push("switchable_models_filtered".to_string());
        }
    } else if let Some(group) = effective_default {
        switchable_model_groups.push(ModelGroupRef {
            id: group.id,
            client_model_name: group.client_model_name.clone(),
        });
    }
    switchable_model_groups.sort_by(|left, right| {
        left.client_model_name
            .cmp(&right.client_model_name)
            .then(left.id.cmp(&right.id))
    });
    warnings.sort();
    warnings.dedup();

    ToolDefaultView {
        tool_key: default.tool_key.clone(),
        scope_type: default.scope_type.clone(),
        default_model_group_id: effective_default.map(|group| group.id),
        default_model_client_name: effective_default.map(|group| group.client_model_name.clone()),
        allow_switch: default.allow_switch,
        switchable_model_groups,
        warnings,
    }
}

/// 在已构建的投影上解析生效的工具默认（不做 IO；preview/tests 复用）。
fn resolve_tool_default_views(
    projection: &Projection,
    subject: &CatalogSubject,
) -> Vec<ToolDefaultView> {
    let mut chosen: HashMap<String, &ProjectionToolDefault> = HashMap::new();
    for default in &projection.tool_defaults {
        let applicable = match default.scope_type.as_str() {
            "global" => true,
            "user" => default.scope_id == Some(subject.user_id),
            "user_group" => default
                .scope_id
                .is_some_and(|group_id| subject.user_group_ids.contains(&group_id)),
            // application 等未来扩展：未实现的层级 fail-closed。
            _ => false,
        };
        if !applicable {
            continue;
        }
        let rank = tool_scope_rank(&default.scope_type);
        match chosen.get(&default.tool_key) {
            Some(existing) => {
                let existing_rank = tool_scope_rank(&existing.scope_type);
                // 同层多组命中（仅 user_group）：确定性取 updated_at DESC, id ASC。
                let replace = if rank > existing_rank {
                    true
                } else if rank == existing_rank && default.scope_type == "user_group" {
                    default.updated_at > existing.updated_at
                        || (default.updated_at == existing.updated_at && default.id < existing.id)
                } else {
                    false
                };
                if replace {
                    chosen.insert(default.tool_key.clone(), default);
                }
            }
            _ => {
                chosen.insert(default.tool_key.clone(), default);
            }
        }
    }
    let mut tool_keys: Vec<String> = chosen.keys().cloned().collect();
    tool_keys.sort();
    tool_keys
        .into_iter()
        .filter_map(|tool_key| {
            chosen
                .remove(&tool_key)
                .map(|default| build_tool_default_view(default, projection, subject))
        })
        .collect()
}

/// 按 subject 解析生效的工具默认模型（user > global；默认/可切换必须可见且可调用）。
///
/// 注：客户端默认模型 HTTP 端点后续 iteration 接线；本轮先固化内部接口与语义。
#[allow(dead_code)]
pub async fn tool_defaults_for(
    state: &AppState,
    subject: &CatalogSubject,
) -> Result<Vec<ToolDefaultView>, CatalogError> {
    let projection = load_projection(state).await.map_err(catalog_db_error)?;
    Ok(resolve_tool_default_views(&projection, subject))
}

// ===========================================================================
// 员工目录预览（AdminSession，只读诊断；迭代 5）
// ===========================================================================

fn preview_not_found(code: &'static str, detail: &'static str) -> AppError {
    AppError::not_found(code, detail)
}

/// 管理台员工目录预览：按 user/device/key 投影出「该员工实际看到什么」。
///
/// - 只读：不调上游、不写 usage/请求日志（管理员诊断）；
/// - 与客户端 `/v1/models` 同源投影（同一次 RR 读取），补 visible/allowed/matched_by/publication_id 诊断；
/// - 只返回可见模型；不返回 canonical/价格/供应商/账号。
pub async fn admin_catalog_preview(
    state: &AppState,
    query: CatalogPreviewQuery,
) -> Result<AdminCatalogPreview, AppError> {
    let user = access_service::get_user_by_id(state, query.user_id)
        .await?
        .ok_or_else(|| preview_not_found("user_not_found", "员工用户不存在"))?;
    let device = match query.device_id {
        Some(device_id) => {
            let detail = access_service::get_device(state, device_id).await?;
            if detail.device.user.id != query.user_id {
                return Err(preview_not_found("device_not_found", "设备不存在"));
            }
            Some(detail.device)
        }
        None => None,
    };
    let key = match query.key_id {
        Some(key_id) => {
            let key = access_service::get_key(state, key_id).await?;
            if key.user.id != query.user_id {
                return Err(preview_not_found(
                    "key_not_found",
                    "Key 不存在或不属于该用户",
                ));
            }
            if let Some(device) = &device
                && key.device.id != device.id
            {
                return Err(preview_not_found(
                    "key_not_found",
                    "Key 不存在或与该设备不匹配",
                ));
            }
            Some(key)
        }
        None => None,
    };

    let user_group_ids = access_service::user_group_ids(state, query.user_id).await?;
    let subject = CatalogSubject {
        user_id: query.user_id,
        device_id: query.device_id,
        user_group_ids: user_group_ids.clone(),
        key_allowed_models: key
            .as_ref()
            .map(|key| key.allowed_models.clone())
            .unwrap_or_default(),
    };
    let projection = load_projection(state).await?;

    let mut models = Vec::new();
    for group in &projection.groups {
        let Some((effective, matched_by)) = model::effective_publication_with_match(
            &group.targets,
            &subject,
            &group.client_model_name,
        ) else {
            continue;
        };
        if !effective.visible {
            continue;
        }
        models.push(AdminPreviewModel {
            model: build_client_model(group, &projection.upstream),
            visible: true,
            allowed: effective.allowed,
            matched_by: Some(matched_by.to_string()),
            publication_id: group.publication_id,
        });
    }

    let tool_views = resolve_tool_default_views(&projection, &subject);
    let tool_defaults = tool_views
        .iter()
        .map(|view| AdminPreviewToolDefault {
            tool_key: view.tool_key.clone(),
            scope_type: view.scope_type.clone(),
            source: view.scope_type.clone(),
            default_model: view.default_model_client_name.clone(),
            switchable_models: view
                .switchable_model_groups
                .iter()
                .map(|group| group.client_model_name.clone())
                .collect(),
            warnings: view.warnings.clone(),
        })
        .collect();

    let mut warnings: Vec<String> = Vec::new();
    if models.is_empty() {
        warnings.push("no_visible_models".to_string());
    }
    if user.status != "active" {
        warnings.push("user_disabled".to_string());
    }
    if let Some(device) = &device
        && device.status != "active"
    {
        warnings.push("device_disabled".to_string());
    }
    if let Some(key) = &key {
        if key.status != "active" {
            warnings.push("key_inactive".to_string());
        }
        if key
            .expires_at
            .is_some_and(|expires_at| expires_at <= chrono::Utc::now())
        {
            warnings.push("key_expired".to_string());
        }
    }
    for view in &tool_views {
        warnings.extend(view.warnings.clone());
    }
    warnings.sort();
    warnings.dedup();

    let key_allowed_models = key
        .as_ref()
        .map(|key| key.allowed_models.clone())
        .unwrap_or_default();
    Ok(AdminCatalogPreview {
        catalog_version: projection.version.clone(),
        subject: PreviewSubject {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            user_status: user.status,
            device_id: query.device_id,
            device_name: device.map(|device| device.name),
            key_id: key.as_ref().map(|key| key.id),
            key_allowed_models,
            user_group_ids,
        },
        models,
        tool_defaults,
        enforcement: model::Enforcement::default(),
        warnings,
    })
}

// ===========================================================================
// 聚合建议（补全）：实时计算 + confirm/reject（决策落表）
// ===========================================================================

struct SeenSuggestion {
    seen_at: StdInstant,
}

const SEEN_TTL_SECS: u64 = 600;
const SEEN_MAX_ENTRIES: usize = 1000;

fn seen_registry() -> &'static Mutex<HashMap<String, SeenSuggestion>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, SeenSuggestion>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn seen_lock() -> std::sync::MutexGuard<'static, HashMap<String, SeenSuggestion>> {
    match seen_registry().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn remember_suggestions(suggestions: &[MergeSuggestion]) {
    let mut registry = seen_lock();
    registry.retain(|_, seen| seen.seen_at.elapsed().as_secs() < SEEN_TTL_SECS);
    if registry.len() + suggestions.len() > SEEN_MAX_ENTRIES {
        registry.clear();
    }
    for suggestion in suggestions {
        registry.insert(
            suggestion.suggestion_key.clone(),
            SeenSuggestion {
                seen_at: StdInstant::now(),
            },
        );
    }
}

fn seen_before(suggestion_key: &str) -> bool {
    seen_lock().contains_key(suggestion_key)
}

fn suggestion_not_found() -> AppError {
    AppError::not_found("suggestion_not_found", "聚合建议不存在或已失效，请刷新列表")
}

fn suggestion_stale() -> AppError {
    AppError::conflict(
        "suggestion_stale",
        "建议的部署集合已变化，请刷新后重试",
        None,
    )
}

fn suggestion_not_mergeable() -> AppError {
    AppError::bad_request(
        "suggestion_not_mergeable",
        "该建议为保持独立（版本/变体不同），不能确认合并",
    )
}

fn validate_suggestion_key(raw: &str) -> Result<(), AppError> {
    if raw.len() == 32 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(suggestion_not_found())
    }
}

fn build_suggestion_deployment(
    row: &DeploymentRow,
    views: &ProviderViews,
) -> Result<SuggestionDeployment, AppError> {
    let upstream = upstream_view(views, row.upstream_model_id)?;
    let account = account_view(views, row.provider_account_id);
    Ok(SuggestionDeployment {
        id: row.id,
        upstream_model_id: row.upstream_model_id,
        upstream_model: upstream.upstream_model.clone(),
        canonical: upstream.canonical.clone(),
        provider_id: upstream.provider_id,
        provider_name: upstream.provider_name.clone(),
        provider_kind: upstream.provider_kind.clone(),
        provider_account_id: row.provider_account_id,
        account_name: account
            .map(|account| account.name.clone())
            .unwrap_or_default(),
        account_region: account
            .map(|account| account.region.clone())
            .unwrap_or_default(),
        priority: row.priority,
        weight: row.weight,
        enabled: row.enabled,
        price_input: upstream.price_input,
        price_output: upstream.price_output,
        currency: upstream.currency.clone(),
        context_window: upstream.context_window,
        max_output: upstream.max_output,
        capabilities: upstream.capabilities.clone(),
    })
}

fn push_diff(
    diff: &mut Vec<DiffItem>,
    risk_flags: &mut Vec<String>,
    field: &str,
    risk_flag: &str,
    values: Vec<DiffValue>,
    note: &str,
) {
    diff.push(DiffItem {
        field: field.to_string(),
        risk_flag: risk_flag.to_string(),
        values,
        note: note.to_string(),
    });
    if !risk_flags.iter().any(|flag| flag == risk_flag) {
        risk_flags.push(risk_flag.to_string());
    }
}

fn distinct_strings(values: &[String]) -> Vec<String> {
    let mut distinct: Vec<String> = values.to_vec();
    distinct.sort();
    distinct.dedup();
    distinct
}

fn build_suggestion_diff(deployments: &[SuggestionDeployment]) -> (Vec<DiffItem>, Vec<String>) {
    let mut diff = Vec::new();
    let mut risk_flags = Vec::new();

    let names: Vec<String> = deployments
        .iter()
        .map(|deployment| deployment.upstream_model.clone())
        .collect();
    if distinct_strings(&names).len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "upstream_model_name",
            "variant_mismatch",
            deployments
                .iter()
                .map(|deployment| DiffValue {
                    deployment_id: deployment.id,
                    value: deployment.upstream_model.clone(),
                })
                .collect(),
            "上游模型名（版本/变体代理）不同：不自动合并，建议保持独立",
        );
    }

    let provider_ids: HashSet<Uuid> = deployments
        .iter()
        .map(|deployment| deployment.provider_id)
        .collect();
    if provider_ids.len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "provider",
            "multi_provider",
            deployments
                .iter()
                .map(|deployment| DiffValue {
                    deployment_id: deployment.id,
                    value: deployment.provider_name.clone(),
                })
                .collect(),
            "多供应商来源（信息项，不阻止合并）",
        );
    }

    let account_ids: HashSet<Uuid> = deployments
        .iter()
        .map(|deployment| deployment.provider_account_id)
        .collect();
    if account_ids.len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "account",
            "multi_account",
            deployments
                .iter()
                .map(|deployment| DiffValue {
                    deployment_id: deployment.id,
                    value: deployment.account_name.clone(),
                })
                .collect(),
            "多渠道账号来源（信息项，不阻止合并）",
        );
    }

    let capabilities: Vec<String> = deployments
        .iter()
        .map(|deployment| {
            let mut caps = deployment.capabilities.clone();
            caps.sort();
            caps.join(",")
        })
        .collect();
    if distinct_strings(&capabilities).len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "capabilities",
            "capability_mismatch",
            deployments
                .iter()
                .zip(capabilities.iter())
                .map(|(deployment, value)| DiffValue {
                    deployment_id: deployment.id,
                    value: value.clone(),
                })
                .collect(),
            "能力标签不一致（组未配置时按交集推导）",
        );
    }

    let context_values: Vec<String> = deployments
        .iter()
        .map(|deployment| {
            deployment
                .context_window
                .map_or_else(|| "null".to_string(), |value| value.to_string())
        })
        .collect();
    if distinct_strings(&context_values).len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "context_window",
            "context_mismatch",
            deployments
                .iter()
                .zip(context_values.iter())
                .map(|(deployment, value)| DiffValue {
                    deployment_id: deployment.id,
                    value: value.clone(),
                })
                .collect(),
            "上下文上限不一致（组未配置时取最小值）",
        );
    }

    let output_values: Vec<String> = deployments
        .iter()
        .map(|deployment| {
            deployment
                .max_output
                .map_or_else(|| "null".to_string(), |value| value.to_string())
        })
        .collect();
    if distinct_strings(&output_values).len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "max_output",
            "context_mismatch",
            deployments
                .iter()
                .zip(output_values.iter())
                .map(|(deployment, value)| DiffValue {
                    deployment_id: deployment.id,
                    value: value.clone(),
                })
                .collect(),
            "输出上限不一致（组未配置时取最小值）",
        );
    }

    for (field, getter) in [
        (
            "price_input",
            (|deployment: &SuggestionDeployment| deployment.price_input)
                as fn(&SuggestionDeployment) -> Option<f64>,
        ),
        (
            "price_output",
            (|deployment: &SuggestionDeployment| deployment.price_output)
                as fn(&SuggestionDeployment) -> Option<f64>,
        ),
    ] {
        let values: Vec<String> = deployments
            .iter()
            .map(|deployment| {
                getter(deployment).map_or_else(|| "null".to_string(), |value| format!("{value:.6}"))
            })
            .collect();
        if distinct_strings(&values).len() > 1 {
            push_diff(
                &mut diff,
                &mut risk_flags,
                field,
                "price_mismatch",
                deployments
                    .iter()
                    .zip(values.iter())
                    .map(|(deployment, value)| DiffValue {
                        deployment_id: deployment.id,
                        value: value.clone(),
                    })
                    .collect(),
                "上游价目不一致（仅管理台可见；不阻止合并）",
            );
        }
    }

    let currencies: Vec<String> = deployments
        .iter()
        .map(|deployment| deployment.currency.clone())
        .collect();
    if distinct_strings(&currencies).len() > 1 {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "currency",
            "currency_mismatch",
            deployments
                .iter()
                .zip(currencies.iter())
                .map(|(deployment, value)| DiffValue {
                    deployment_id: deployment.id,
                    value: value.clone(),
                })
                .collect(),
            "币种不一致（成本口径需注意）",
        );
    }

    if deployments.iter().any(|deployment| !deployment.enabled) {
        push_diff(
            &mut diff,
            &mut risk_flags,
            "enabled",
            "disabled_deployment",
            deployments
                .iter()
                .map(|deployment| DiffValue {
                    deployment_id: deployment.id,
                    value: deployment.enabled.to_string(),
                })
                .collect(),
            "存在停用部署（入组后不参与路由）",
        );
    }

    risk_flags.sort();
    risk_flags.dedup();
    (diff, risk_flags)
}

async fn build_suggestion(
    pool: &PgPool,
    canonical: &str,
    rows: &[&DeploymentRow],
    views: &ProviderViews,
) -> Result<MergeSuggestion, AppError> {
    let mut deployments = rows
        .iter()
        .map(|row| build_suggestion_deployment(row, views))
        .collect::<Result<Vec<_>, _>>()?;
    deployments.sort_by_key(|deployment| deployment.id);

    let mut names: Vec<String> = deployments
        .iter()
        .map(|deployment| deployment.upstream_model.clone())
        .collect();
    names.sort();
    names.dedup();
    let deployment_ids: Vec<Uuid> = deployments.iter().map(|deployment| deployment.id).collect();
    let suggestion_key = model::suggestion_key(canonical, &names, &deployment_ids);
    let recommendation = if names.len() == 1 {
        "merge"
    } else {
        "keep_separate"
    };
    let (diff, risk_flags) = build_suggestion_diff(&deployments);
    let provider_count = deployments
        .iter()
        .map(|deployment| deployment.provider_id)
        .collect::<HashSet<_>>()
        .len() as i64;
    let first_seen_at = rows
        .iter()
        .map(|row| row.created_at)
        .min()
        .unwrap_or_else(chrono::Utc::now);
    let related_groups =
        repo::list_groups_by_canonical(pool, canonical, model::MAX_RELATED_GROUPS as i64)
            .await?
            .into_iter()
            .filter(|group| group.status != model::GROUP_STATUS_ARCHIVED)
            .map(|group| RelatedGroup {
                id: group.id,
                client_model_name: group.client_model_name,
                status: group.status,
            })
            .collect();

    Ok(MergeSuggestion {
        suggestion_key,
        canonical: canonical.to_string(),
        upstream_model_name: names.join(" / "),
        recommendation,
        deployment_count: deployments.len() as i64,
        provider_count,
        deployments,
        related_groups,
        diff,
        risk_flags,
        first_seen_at,
    })
}

async fn compute_suggestions(
    state: &AppState,
    canonical_filter: Option<&str>,
) -> Result<Vec<MergeSuggestion>, AppError> {
    let pool = db(state)?;
    let ungrouped = repo::list_ungrouped_deployments(pool).await?;
    if ungrouped.is_empty() {
        return Ok(Vec::new());
    }
    let views = load_provider_views(state).await?;
    let mut by_canonical: HashMap<String, Vec<&DeploymentRow>> = HashMap::new();
    for row in &ungrouped {
        if let Some(upstream) = views.upstream.get(&row.upstream_model_id) {
            by_canonical
                .entry(upstream.canonical.clone())
                .or_default()
                .push(row);
        }
    }
    let mut suggestions = Vec::new();
    for (canonical, rows) in by_canonical {
        if rows.len() < 2 {
            continue;
        }
        if canonical_filter.is_some_and(|filter| filter != canonical) {
            continue;
        }
        suggestions.push(build_suggestion(pool, &canonical, &rows, &views).await?);
    }
    let keys: Vec<String> = suggestions
        .iter()
        .map(|suggestion| suggestion.suggestion_key.clone())
        .collect();
    let rejected = repo::rejected_suggestion_keys(pool, &keys).await?;
    suggestions.retain(|suggestion| !rejected.contains(&suggestion.suggestion_key));
    suggestions.sort_by(|left, right| {
        left.first_seen_at
            .cmp(&right.first_seen_at)
            .then(left.suggestion_key.cmp(&right.suggestion_key))
    });
    remember_suggestions(&suggestions);
    Ok(suggestions)
}

pub async fn list_model_group_suggestions(
    state: &AppState,
    query: SuggestionListQuery,
) -> Result<Page<MergeSuggestion>, AppError> {
    let page = model::validate_page(query.limit, query.offset)?;
    let (canonical, _) = model::validate_list_filters(query.canonical.as_deref(), None)?;
    let mut suggestions = compute_suggestions(state, canonical.as_deref()).await?;
    let total = suggestions.len() as i64;
    let max = model::MAX_SUGGESTIONS;
    let start = usize::try_from(page.offset).unwrap_or(0).min(max);
    let take = usize::try_from(page.limit).unwrap_or(0).min(max - start);
    let items: Vec<MergeSuggestion> = suggestions.drain(..).skip(start).take(take).collect();
    Ok(Page {
        items,
        total,
        limit: page.limit,
        offset: page.offset,
    })
}

fn group_fields_from_suggestion(
    suggestion: &MergeSuggestion,
    client_model_name: String,
    display_name: String,
) -> GroupFields {
    let enabled: Vec<&SuggestionDeployment> = suggestion
        .deployments
        .iter()
        .filter(|deployment| deployment.enabled)
        .collect();
    let capability_sets: Vec<Vec<String>> = enabled
        .iter()
        .map(|deployment| deployment.capabilities.clone())
        .collect();
    let capabilities = model::derive_capabilities(&[], &capability_sets);
    let context_values: Vec<Option<i32>> = enabled
        .iter()
        .map(|deployment| deployment.context_window)
        .collect();
    let output_values: Vec<Option<i32>> = enabled
        .iter()
        .map(|deployment| deployment.max_output)
        .collect();
    let context_window = model::derive_min_metric(None, &context_values);
    let max_output = model::derive_min_metric(None, &output_values);
    let currency = suggestion
        .deployments
        .first()
        .map(|deployment| deployment.currency.clone())
        .filter(|currency| {
            suggestion
                .deployments
                .iter()
                .all(|deployment| &deployment.currency == currency)
        })
        .unwrap_or_else(|| "CNY".to_string());
    let status = if enabled.is_empty() {
        model::GROUP_STATUS_DRAFT
    } else {
        model::GROUP_STATUS_ACTIVE
    };
    GroupFields {
        client_model_name,
        display_name,
        canonical: suggestion.canonical.clone(),
        description: String::new(),
        capabilities,
        context_window,
        max_output,
        protocols: Vec::new(),
        parameters: serde_json::Value::Object(serde_json::Map::new()),
        price_input: None,
        price_output: None,
        currency,
        status: status.to_string(),
    }
}

async fn confirmation_from_decision(
    state: &AppState,
    decision: MergeDecisionRow,
) -> Result<MergeConfirmation, AppError> {
    let Some(group_id) = decision.model_group_id else {
        return Err(suggestion_not_found());
    };
    let group = get_model_group(state, group_id).await?;
    Ok(MergeConfirmation {
        suggestion_key: decision.suggestion_key,
        decision: "merged",
        group,
        moved_deployment_ids: decision.deployment_ids,
        decided_at: decision.created_at,
        decided_by: decision.decided_by,
    })
}

fn rejection_from_decision(decision: MergeDecisionRow) -> MergeRejection {
    MergeRejection {
        suggestion_key: decision.suggestion_key,
        decision: "rejected",
        note: decision.note,
        decided_at: decision.created_at,
        decided_by: decision.decided_by,
    }
}

pub async fn confirm_model_group_suggestion(
    state: &AppState,
    suggestion_key: &str,
    request: ConfirmMergeSuggestionRequest,
    decided_by: Uuid,
) -> Result<MergeConfirmation, AppError> {
    validate_suggestion_key(suggestion_key)?;
    let action = model::validate_confirm_suggestion_request(request)?;
    let pool = db(state)?;
    if let Some(decision) = repo::get_merge_decision(pool, suggestion_key).await? {
        return match decision.decision.as_str() {
            "merged" => confirmation_from_decision(state, decision).await,
            _ => Err(suggestion_not_found()),
        };
    }

    let Some(suggestion) = compute_suggestions(state, None)
        .await?
        .into_iter()
        .find(|suggestion| suggestion.suggestion_key == suggestion_key)
    else {
        return Err(if seen_before(suggestion_key) {
            suggestion_stale()
        } else {
            suggestion_not_found()
        });
    };
    if suggestion.recommendation != "merge" {
        return Err(suggestion_not_mergeable());
    }

    let deployment_ids: Vec<Uuid> = suggestion
        .deployments
        .iter()
        .map(|deployment| deployment.id)
        .collect();
    let mut tx = pool.begin().await?;
    let created_group_id: Option<Uuid>;
    let model_group_id = match action {
        MergeConfirmAction::CreateGroup {
            client_model_name,
            display_name,
        } => {
            let fields = group_fields_from_suggestion(&suggestion, client_model_name, display_name);
            let group_id = Uuid::new_v4();
            repo::insert_model_group(&mut tx, group_id, &fields).await?;
            created_group_id = Some(group_id);
            group_id
        }
        MergeConfirmAction::MergeIntoGroup { target_group_id } => {
            let group = repo::get_model_group_conn(&mut tx, target_group_id)
                .await?
                .ok_or_else(group_not_found)?;
            if group.status == model::GROUP_STATUS_ARCHIVED {
                tx.rollback().await.ok();
                return Err(AppError::bad_request(
                    "model_group_not_active",
                    "目标模型组已归档，不能合并",
                ));
            }
            if group.canonical != suggestion.canonical {
                tx.rollback().await.ok();
                return Err(AppError::bad_request(
                    "canonical_mismatch",
                    "目标模型组 canonical 与建议不一致",
                ));
            }
            created_group_id = None;
            target_group_id
        }
    };
    let attached = repo::attach_deployments(&mut tx, &deployment_ids, model_group_id).await?;
    if attached != deployment_ids.len() as u64 {
        tx.rollback().await.ok();
        return Err(suggestion_stale());
    }
    repo::touch_model_group(&mut tx, model_group_id).await?;
    let diff_value = serde_json::to_value(&suggestion.diff).map_err(AppError::internal)?;
    let inserted = repo::insert_merge_decision(
        &mut tx,
        &NewMergeDecision {
            suggestion_key: &suggestion.suggestion_key,
            canonical: &suggestion.canonical,
            upstream_model_name: &suggestion.upstream_model_name,
            decision: "merged",
            model_group_id: Some(model_group_id),
            deployment_ids: &deployment_ids,
            diff: &diff_value,
            decided_by: Some(decided_by),
            note: "",
        },
    )
    .await?;
    if !inserted {
        tx.rollback().await.ok();
        if let Some(decision) = repo::get_merge_decision(pool, suggestion_key).await? {
            return match decision.decision.as_str() {
                "merged" => confirmation_from_decision(state, decision).await,
                _ => Err(suggestion_stale()),
            };
        }
        return Err(suggestion_stale());
    }
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    let _ = created_group_id;
    let decision = repo::get_merge_decision(pool, suggestion_key)
        .await?
        .ok_or_else(suggestion_not_found)?;
    confirmation_from_decision(state, decision).await
}

pub async fn reject_model_group_suggestion(
    state: &AppState,
    suggestion_key: &str,
    request: RejectMergeSuggestionRequest,
    decided_by: Uuid,
) -> Result<MergeRejection, AppError> {
    validate_suggestion_key(suggestion_key)?;
    let note = model::validate_reject_suggestion_request(request)?;
    let pool = db(state)?;
    if let Some(decision) = repo::get_merge_decision(pool, suggestion_key).await? {
        return match decision.decision.as_str() {
            "rejected" => Ok(rejection_from_decision(decision)),
            _ => Err(suggestion_stale()),
        };
    }
    let Some(suggestion) = compute_suggestions(state, None)
        .await?
        .into_iter()
        .find(|suggestion| suggestion.suggestion_key == suggestion_key)
    else {
        return Err(if seen_before(suggestion_key) {
            suggestion_stale()
        } else {
            suggestion_not_found()
        });
    };
    let deployment_ids: Vec<Uuid> = suggestion
        .deployments
        .iter()
        .map(|deployment| deployment.id)
        .collect();
    let diff_value = serde_json::to_value(&suggestion.diff).map_err(AppError::internal)?;
    let mut tx = pool.begin().await?;
    let inserted = repo::insert_merge_decision(
        &mut tx,
        &NewMergeDecision {
            suggestion_key: &suggestion.suggestion_key,
            canonical: &suggestion.canonical,
            upstream_model_name: &suggestion.upstream_model_name,
            decision: "rejected",
            model_group_id: None,
            deployment_ids: &deployment_ids,
            diff: &diff_value,
            decided_by: Some(decided_by),
            note: &note,
        },
    )
    .await?;
    if !inserted {
        tx.rollback().await.ok();
        if let Some(decision) = repo::get_merge_decision(pool, suggestion_key).await? {
            return match decision.decision.as_str() {
                "rejected" => Ok(rejection_from_decision(decision)),
                _ => Err(suggestion_stale()),
            };
        }
        return Err(suggestion_stale());
    }
    repo::bump_version(&mut tx).await?;
    tx.commit().await?;
    invalidate();
    let decision = repo::get_merge_decision(pool, suggestion_key)
        .await?
        .ok_or_else(suggestion_not_found)?;
    Ok(rejection_from_decision(decision))
}

// ===========================================================================
// 客户端每工具默认模型（GET /v1/model-defaults）
// ===========================================================================

/// 客户端默认模型投影：只返回「有效默认」的工具；switchable 为客户端模型名。
pub async fn client_model_defaults(
    state: &AppState,
    subject: &CatalogSubject,
) -> Result<ClientModelDefaultsList, AppError> {
    let projection = load_projection(state).await?;
    let data = resolve_tool_default_views(&projection, subject)
        .into_iter()
        .filter_map(|view| {
            let default_model = view.default_model_client_name?;
            Some(ClientModelDefault {
                tool_key: view.tool_key,
                default_model,
                allow_switch: view.allow_switch,
                switchable_models: view
                    .switchable_model_groups
                    .iter()
                    .map(|group| group.client_model_name.clone())
                    .collect(),
            })
        })
        .collect();
    Ok(ClientModelDefaultsList {
        object: "list",
        data,
        catalog_version: projection.version.clone(),
    })
}

// ===========================================================================
// gated 自测（需要真实 Postgres；默认忽略）
//
// 运行：
// `GW_TEST_DATABASE_URL=postgres://gw:gw@127.0.0.1:5432/gw_test \
//  cargo test --all-targets -- --ignored --test-threads=1`
// ===========================================================================
#[cfg(test)]
mod gated_db_tests {
    use std::sync::Arc;

    use serde_json::json;
    use sqlx::PgPool;
    use uuid::Uuid;

    use super::*;
    use crate::platform::{config::Config, crypto::Crypto};

    async fn test_state() -> Option<(AppState, PgPool)> {
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
        sqlx::query(
            "TRUNCATE admin_users, users, providers, model_groups, publications, deployments, user_groups CASCADE",
        )
        .execute(&pool)
        .await
        .expect("truncate catalog test tables");
        // 缓存跨用例隔离：清空投影/routing 视图缓存并恢复测试默认 TTL（0=关闭缓存）。
        clear_caches();
        set_snapshot_ttl_for_test(crate::modules::catalog::cache::DEFAULT_TTL);
        let config = Config {
            bind: "127.0.0.1:0".parse().expect("bind addr"),
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
        let state = AppState {
            config: Arc::new(config),
            db: Some(pool.clone()),
            crypto: Arc::new(Crypto::from_key_bytes([3u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: std::time::Instant::now(),
        };
        Some((state, pool))
    }

    struct Fixture {
        provider_id: Uuid,
        account_id: Uuid,
        admin_id: Uuid,
        alice_id: Uuid,
        bob_id: Uuid,
    }

    async fn seed(pool: &PgPool) -> Fixture {
        let provider_id = Uuid::new_v4();
        let account_id = Uuid::new_v4();
        let admin_id = Uuid::new_v4();
        let alice_id = Uuid::new_v4();
        let bob_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO providers (id, name, kind, base_url) VALUES ($1, 'P', 'openai_compatible', 'https://p.example/v1')",
        )
        .bind(provider_id)
        .execute(pool)
        .await
        .expect("insert provider");
        sqlx::query(
            "INSERT INTO provider_accounts (id, provider_id, name, credential_encrypted) \
             VALUES ($1, $2, 'acct', '\\x01'::bytea)",
        )
        .bind(account_id)
        .bind(provider_id)
        .execute(pool)
        .await
        .expect("insert account");
        sqlx::query(
            "INSERT INTO admin_users (id, email, password_hash) VALUES ($1, 'gated-admin@corp.example.com', 'x')",
        )
        .bind(admin_id)
        .execute(pool)
        .await
        .expect("insert admin");
        sqlx::query(
            "INSERT INTO users (id, email, display_name) VALUES ($1, 'alice@corp.example.com', 'Alice'), \
             ($2, 'bob@corp.example.com', 'Bob')",
        )
        .bind(alice_id)
        .bind(bob_id)
        .execute(pool)
        .await
        .expect("insert users");
        Fixture {
            provider_id,
            account_id,
            admin_id,
            alice_id,
            bob_id,
        }
    }

    async fn insert_upstream(
        pool: &PgPool,
        provider_id: Uuid,
        name: &str,
        canonical: &str,
    ) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO upstream_models \
             (id, provider_id, name, canonical, capabilities, context_window, max_output, \
              price_input, price_output, currency) \
             VALUES ($1, $2, $3, $4, '[\"chat\"]'::jsonb, 1000, 100, 0.001, 0.002, 'CNY')",
        )
        .bind(id)
        .bind(provider_id)
        .bind(name)
        .bind(canonical)
        .execute(pool)
        .await
        .expect("insert upstream model");
        id
    }

    fn group_request(
        client_model_name: &str,
        canonical: &str,
        upstream_model_id: Uuid,
        account_id: Uuid,
        status: &str,
    ) -> ModelGroupCreateRequest {
        serde_json::from_value(json!({
            "client_model_name": client_model_name,
            "display_name": format!("{client_model_name} display"),
            "canonical": canonical,
            "capabilities": [],
            "protocols": ["openai_chat"],
            "parameters": {"temperature": true},
            "status": status,
            "deployments": [{
                "upstream_model_id": upstream_model_id,
                "provider_account_id": account_id
            }]
        }))
        .expect("group request")
    }

    fn subject(user_id: Uuid, whitelist: &[&str]) -> CatalogSubject {
        CatalogSubject {
            user_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: whitelist.iter().map(|item| item.to_string()).collect(),
        }
    }

    fn publication_request(
        model_group_id: Uuid,
        targets: serde_json::Value,
    ) -> PublicationCreateRequest {
        serde_json::from_value(json!({
            "model_group_id": model_group_id,
            "data_level_max": 2,
            "quota_rpm": 60,
            "quota_tpm": 200000,
            "note": "gated",
            "targets": targets
        }))
        .expect("publication request")
    }

    fn update_request(value: serde_json::Value) -> PublicationUpdateRequest {
        serde_json::from_value(value).expect("publication update request")
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_publication_projection_authorize() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let upstream_one = insert_upstream(&pool, fixture.provider_id, "m1", "c1").await;
        let upstream_three = insert_upstream(&pool, fixture.provider_id, "m3", "c1").await;

        // active 组（含启用部署）
        let group_one = create_model_group(
            &state,
            group_request(
                "m-one",
                "c1",
                upstream_one,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create group one");
        // draft 组（无启用部署，不能发布）
        let group_draft = create_model_group(
            &state,
            serde_json::from_value(json!({
                "client_model_name": "m-draft",
                "canonical": "c1"
            }))
            .expect("draft group request"),
        )
        .await
        .expect("create draft group");
        // active 组 3（用于 user 定向发布）
        let group_three = create_model_group(
            &state,
            group_request(
                "m-three",
                "c1",
                upstream_three,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create group three");
        let _ = group_three;

        let before_version = repo::read_projection_state(&pool)
            .await
            .expect("read version")
            .version;

        // ---- 发布创建：all_users（group one）
        let publication = create_publication(
            &state,
            publication_request(group_one.summary.id, json!([{"subject_type": "all_users"}])),
            fixture.admin_id,
        )
        .await
        .expect("create publication");
        assert_eq!(publication.status, "active");
        assert_eq!(publication.model_group_client_name, "m-one");
        assert_eq!(publication.created_by, Some(fixture.admin_id));
        assert_eq!(publication.enforcement.visible, "enforced");
        assert_eq!(publication.enforcement.quota, "config_only");
        assert_eq!(publication.targets.len(), 1);
        assert_eq!(
            publication.targets[0].subject_label.as_deref(),
            Some("全部用户")
        );
        let after_version = repo::read_projection_state(&pool)
            .await
            .expect("read version")
            .version;
        assert_eq!(
            after_version,
            before_version + 1,
            "publication create must bump version"
        );

        // 1:1 冲突
        let conflict = create_publication(
            &state,
            publication_request(group_one.summary.id, json!([{"subject_type": "all_users"}])),
            fixture.admin_id,
        )
        .await
        .expect_err("duplicate publication must conflict");
        assert!(
            matches!(
                conflict,
                AppError::Conflict {
                    code: "publication_conflict",
                    ..
                }
            ),
            "unexpected conflict error: {conflict:?}"
        );

        // 未就绪
        let not_ready = create_publication(
            &state,
            publication_request(
                group_draft.summary.id,
                json!([{"subject_type": "all_users"}]),
            ),
            fixture.admin_id,
        )
        .await
        .expect_err("draft group must not be ready");
        assert!(
            matches!(
                not_ready,
                AppError::BadRequest {
                    code: "model_group_not_ready",
                    ..
                }
            ),
            "unexpected not-ready error: {not_ready:?}"
        );

        // 预留类型 / 未知用户
        let reserved = create_publication(
            &state,
            publication_request(
                group_draft.summary.id,
                json!([{"subject_type": "application", "subject_id": Uuid::new_v4()}]),
            ),
            fixture.admin_id,
        )
        .await
        .expect_err("reserved target type must be rejected");
        assert!(
            matches!(
                reserved,
                AppError::BadRequest {
                    code: "target_type_not_supported",
                    ..
                }
            ),
            "unexpected reserved error: {reserved:?}"
        );
        let unknown_user = create_publication(
            &state,
            publication_request(
                group_one.summary.id,
                json!([{"subject_type": "user", "subject_id": Uuid::new_v4()}]),
            ),
            fixture.admin_id,
        )
        .await
        .expect_err("unknown user target must be rejected");
        assert!(
            matches!(
                unknown_user,
                AppError::NotFound {
                    code: "user_not_found",
                    ..
                }
            ),
            "unexpected unknown-user error: {unknown_user:?}"
        );

        // ---- 客户端列表（all_users）
        let alice = subject(fixture.alice_id, &[]);
        let bob = subject(fixture.bob_id, &[]);
        let catalog = client_catalog(&state, &alice).await.expect("alice catalog");
        assert_eq!(catalog.object, "list");
        assert!(
            catalog.catalog_version.starts_with("cat:"),
            "{}",
            catalog.catalog_version
        );
        assert_eq!(catalog.data.len(), 1, "only published visible active group");
        let model = &catalog.data[0];
        assert_eq!(model.id, "m-one");
        assert_eq!(model.upstream_model.as_deref(), Some("m1"));
        assert_eq!(model.protocols, vec!["openai_chat".to_string()]);
        assert_eq!(
            model.capabilities,
            vec!["chat".to_string()],
            "derived capabilities"
        );
        assert_eq!(model.context_window, Some(1000));
        let serialized = serde_json::to_string(&catalog).expect("serialize catalog");
        for forbidden in [
            "canonical",
            "price",
            "provider",
            "account",
            "allowed",
            "deployment",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "client catalog leaked {forbidden}: {serialized}"
            );
        }

        // ---- 定向发布（user alice → group three）
        let user_publication = create_publication(
            &state,
            publication_request(
                group_three.summary.id,
                json!([{"subject_type": "user", "subject_id": fixture.alice_id}]),
            ),
            fixture.admin_id,
        )
        .await
        .expect("create user publication");
        assert_eq!(
            user_publication.targets[0].subject_label.as_deref(),
            Some("Alice")
        );
        let alice_catalog = client_catalog(&state, &alice).await.expect("alice catalog");
        assert_eq!(
            alice_catalog.data.len(),
            2,
            "alice sees all_users + user target"
        );
        let bob_catalog = client_catalog(&state, &bob).await.expect("bob catalog");
        assert_eq!(bob_catalog.data.len(), 1, "bob sees only all_users group");
        assert_eq!(
            authorize_model(&state, &bob, "m-three")
                .await
                .expect("authorize bob"),
            ModelAuthorization::NotPublished
        );
        assert_eq!(
            authorize_model(&state, &alice, "m-three")
                .await
                .expect("authorize alice"),
            ModelAuthorization::Allowed
        );

        // ---- visible-only
        update_publication(
            &state,
            publication.id,
            update_request(json!({
                "targets": [{"subject_type": "all_users", "visible": true, "allowed": false}]
            })),
        )
        .await
        .expect("visible-only publication");
        assert_eq!(
            authorize_model(&state, &alice, "m-one")
                .await
                .expect("authorize visible-only"),
            ModelAuthorization::VisibleOnly
        );
        let visible_only_catalog = client_catalog(&state, &alice).await.expect("catalog");
        assert!(
            visible_only_catalog
                .data
                .iter()
                .any(|model| model.id == "m-one"),
            "visible-only model must still be listed"
        );

        // ---- Key 白名单叠加
        let denied = subject(fixture.alice_id, &["other-model"]);
        assert_eq!(
            authorize_model(&state, &denied, "m-one")
                .await
                .expect("authorize whitelist"),
            ModelAuthorization::NotPublished
        );
        assert!(
            client_catalog(&state, &denied)
                .await
                .expect("catalog whitelist")
                .data
                .is_empty(),
            "key whitelist must hide all models"
        );

        // ---- 详情 200/404
        let detail = client_model_detail(&state, &alice, "m-one")
            .await
            .expect("detail")
            .expect("visible detail");
        assert_eq!(detail.catalog_version, visible_only_catalog.catalog_version);
        assert_eq!(detail.model.id, "m-one");
        assert!(detail.parameters.get("temperature").is_some());
        assert!(
            client_model_detail(&state, &bob, "m-three")
                .await
                .expect("detail bob")
                .is_none(),
            "not visible detail must be None"
        );
        assert!(
            client_model_detail(&state, &alice, "nope")
                .await
                .expect("detail unknown")
                .is_none()
        );

        // ---- paused 立即不可见
        update_publication(
            &state,
            publication.id,
            update_request(json!({"status": "paused"})),
        )
        .await
        .expect("pause publication");
        assert!(
            client_catalog(&state, &alice)
                .await
                .expect("paused catalog")
                .data
                .iter()
                .all(|model| model.id != "m-one")
        );
        assert_eq!(
            authorize_model(&state, &alice, "m-one")
                .await
                .expect("authorize paused"),
            ModelAuthorization::NotPublished
        );

        // ---- Unknown：不是模型组
        assert_eq!(
            authorize_model(&state, &alice, "no-such-model")
                .await
                .expect("authorize unknown"),
            ModelAuthorization::Unknown
        );

        // ---- 删除发布后回到未发布
        delete_publication(&state, publication.id)
            .await
            .expect("delete publication");
        assert_eq!(
            authorize_model(&state, &alice, "m-one")
                .await
                .expect("authorize after delete"),
            ModelAuthorization::NotPublished
        );
        assert!(
            get_publication(&state, publication.id)
                .await
                .expect_err("deleted publication must 404")
                .to_string()
                .contains("publication_not_found")
        );
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_routing_group_views_and_version() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let upstream = insert_upstream(&pool, fixture.provider_id, "m-routing", "c-routing").await;
        let before = catalog_version(&state).await.expect("version");

        let active = create_model_group(
            &state,
            group_request(
                "m-routing",
                "c-routing",
                upstream,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create active group");
        let draft = create_model_group(
            &state,
            serde_json::from_value(json!({
                "client_model_name": "m-routing-draft",
                "canonical": "c-routing"
            }))
            .expect("draft group request"),
        )
        .await
        .expect("create draft group");
        let _ = draft;

        let after = catalog_version(&state).await.expect("version after");
        assert!(after > before, "catalog version must bump on group create");

        let views = routing_group_views(&state).await.expect("routing views");
        let group = views
            .iter()
            .find(|group| group.client_model_name == "m-routing")
            .expect("active group view");
        assert_eq!(group.group_id, active.summary.id);
        assert_eq!(group.status, model::GROUP_STATUS_ACTIVE);
        assert_eq!(group.deployments.len(), 1);
        let deployment = &group.deployments[0];
        assert_eq!(deployment.provider_account_id, fixture.account_id);
        assert_eq!(
            deployment.provider_id, fixture.provider_id,
            "provider enrichment must be present"
        );
        assert_eq!(deployment.upstream_model, "m-routing");
        assert_eq!(deployment.base_url, "https://p.example/v1");
        assert_eq!(deployment.currency, "CNY");
        assert!(deployment.price_input.is_some());
        // draft 组同样返回（routing 自己过滤 active，draft 不遮蔽 alias）。
        assert!(views.iter().any(|group| {
            group.client_model_name == "m-routing-draft"
                && group.status == model::GROUP_STATUS_DRAFT
        }));
    }

    fn tool_default_create(value: serde_json::Value) -> ToolDefaultCreateRequest {
        serde_json::from_value(value).expect("tool default request")
    }

    fn tool_default_update(value: serde_json::Value) -> ToolDefaultUpdateRequest {
        serde_json::from_value(value).expect("tool default update")
    }

    fn find_view<'a>(views: &'a [ToolDefaultView], tool_key: &str) -> &'a ToolDefaultView {
        views
            .iter()
            .find(|view| view.tool_key == tool_key)
            .unwrap_or_else(|| panic!("missing tool view {tool_key}: {views:?}"))
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_tool_defaults_resolution() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let upstream_one = insert_upstream(&pool, fixture.provider_id, "m1", "c1").await;
        let upstream_two = insert_upstream(&pool, fixture.provider_id, "m2", "c1").await;
        let upstream_three = insert_upstream(&pool, fixture.provider_id, "m3", "c1").await;
        let upstream_four = insert_upstream(&pool, fixture.provider_id, "m4", "c1").await;

        let group_one = create_model_group(
            &state,
            group_request(
                "m-one",
                "c1",
                upstream_one,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create group one");
        let group_two = create_model_group(
            &state,
            group_request(
                "m-two",
                "c1",
                upstream_two,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create group two");
        let group_three = create_model_group(
            &state,
            group_request(
                "m-three",
                "c1",
                upstream_three,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create group three");
        let group_archived = create_model_group(
            &state,
            group_request(
                "m-archived",
                "c1",
                upstream_four,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create archived group");
        archive_model_group(&state, group_archived.summary.id)
            .await
            .expect("archive group");

        // 发布：g1 all_users 可用；g2 all_users 仅可见；g3 仅 alice
        let mut publication_one =
            publication_request(group_one.summary.id, json!([{"subject_type": "all_users"}]));
        publication_one.data_level_max = None;
        let publication_one = create_publication(&state, publication_one, fixture.admin_id)
            .await
            .expect("publication one");
        let mut publication_two = publication_request(
            group_two.summary.id,
            json!([{"subject_type": "all_users", "allowed": false}]),
        );
        publication_two.data_level_max = None;
        let _ = create_publication(&state, publication_two, fixture.admin_id)
            .await
            .expect("publication two");
        let mut publication_three = publication_request(
            group_three.summary.id,
            json!([{"subject_type": "user", "subject_id": fixture.alice_id}]),
        );
        publication_three.data_level_max = None;
        let _ = create_publication(&state, publication_three, fixture.admin_id)
            .await
            .expect("publication three");

        // 校验/错误码：user_group scope 已支持，但组必须存在
        let unknown_group_scope = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "user_group",
                "scope_id": fixture.alice_id,
                "default_model_group_id": group_one.summary.id
            })),
        )
        .await
        .expect_err("unknown user_group scope must be rejected");
        assert!(
            matches!(
                unknown_group_scope,
                AppError::NotFound {
                    code: "user_group_not_found",
                    ..
                }
            ),
            "unexpected unknown group scope: {unknown_group_scope:?}"
        );
        let unknown_user = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "user",
                "scope_id": Uuid::new_v4(),
                "default_model_group_id": group_one.summary.id
            })),
        )
        .await
        .expect_err("unknown user must be rejected");
        assert!(
            matches!(
                unknown_user,
                AppError::NotFound {
                    code: "user_not_found",
                    ..
                }
            ),
            "unexpected unknown user: {unknown_user:?}"
        );
        let not_active = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "global",
                "default_model_group_id": group_archived.summary.id
            })),
        )
        .await
        .expect_err("archived group must be rejected");
        assert!(
            matches!(
                not_active,
                AppError::BadRequest {
                    code: "model_group_not_active",
                    ..
                }
            ),
            "unexpected not-active: {not_active:?}"
        );
        let inactive_whitelist = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "global",
                "default_model_group_id": group_one.summary.id,
                "allowed_model_group_ids": [group_archived.summary.id]
            })),
        )
        .await
        .expect_err("inactive whitelist group must be rejected");
        assert!(
            matches!(
                inactive_whitelist,
                AppError::BadRequest {
                    code: "model_group_not_active",
                    ..
                }
            ),
            "unexpected inactive whitelist: {inactive_whitelist:?}"
        );

        // 全局默认：默认 g1，白名单 g2(仅可见)/g3(仅 alice)
        let global = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "global",
                "scope_id": null,
                "default_model_group_id": group_one.summary.id,
                "allow_switch": true,
                "allowed_model_group_ids": [group_two.summary.id, group_three.summary.id]
            })),
        )
        .await
        .expect("create global tool default");
        assert_eq!(global.scope_label.as_deref(), Some("全部用户"));
        assert_eq!(global.default_model_client_name, "m-one");
        assert_eq!(global.allowed_model_groups.len(), 2);

        let duplicate = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "global",
                "default_model_group_id": group_two.summary.id
            })),
        )
        .await
        .expect_err("duplicate scope must conflict");
        assert!(
            matches!(
                duplicate,
                AppError::Conflict {
                    code: "tool_model_default_conflict",
                    ..
                }
            ),
            "unexpected duplicate: {duplicate:?}"
        );

        // 组详情聚合
        let group_detail = get_model_group(&state, group_one.summary.id)
            .await
            .expect("group detail");
        assert_eq!(
            group_detail.summary.publication_status.as_deref(),
            Some("active")
        );
        assert_eq!(group_detail.publications.len(), 1);
        assert_eq!(group_detail.publications[0].id, publication_one.id);
        assert_eq!(group_detail.summary.tool_default_count, 1);

        // 解析：alice 全局
        let alice = subject(fixture.alice_id, &[]);
        let bob = subject(fixture.bob_id, &[]);
        let alice_views = tool_defaults_for(&state, &alice)
            .await
            .expect("alice defaults");
        let alice_view = find_view(&alice_views, "claude_code");
        assert_eq!(alice_view.scope_type, "global");
        assert_eq!(
            alice_view.default_model_client_name.as_deref(),
            Some("m-one")
        );
        let alice_switchable: Vec<&str> = alice_view
            .switchable_model_groups
            .iter()
            .map(|group| group.client_model_name.as_str())
            .collect();
        assert_eq!(
            alice_switchable,
            vec!["m-one", "m-three"],
            "{alice_switchable:?}"
        );
        assert!(
            alice_view
                .warnings
                .contains(&"switchable_models_filtered".to_string())
        );

        // 解析：bob 全局（g3 定向 alice 不可见）
        let bob_views = tool_defaults_for(&state, &bob).await.expect("bob defaults");
        let bob_view = find_view(&bob_views, "claude_code");
        let bob_switchable: Vec<&str> = bob_view
            .switchable_model_groups
            .iter()
            .map(|group| group.client_model_name.as_str())
            .collect();
        assert_eq!(bob_switchable, vec!["m-one"], "{bob_switchable:?}");

        // user 覆盖：默认 g2（仅可见不可用）→ 无有效默认 + warning
        let user_override = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "user",
                "scope_id": fixture.alice_id,
                "default_model_group_id": group_two.summary.id
            })),
        )
        .await
        .expect("create user override");
        let alice_views = tool_defaults_for(&state, &alice)
            .await
            .expect("alice defaults");
        let alice_view = find_view(&alice_views, "claude_code");
        assert_eq!(alice_view.scope_type, "user");
        assert!(alice_view.default_model_group_id.is_none());
        assert!(
            alice_view
                .warnings
                .contains(&"default_model_not_allowed".to_string())
        );
        assert!(alice_view.switchable_model_groups.is_empty());

        // PATCH：全局关闭切换 + 清空白名单
        let updated = update_tool_model_default(
            &state,
            global.id,
            tool_default_update(json!({"allow_switch": false, "allowed_model_group_ids": []})),
        )
        .await
        .expect("update global default");
        assert!(!updated.allow_switch);
        assert!(updated.allowed_model_groups.is_empty());

        // 删除 user 覆盖后，alice 回落到 global
        delete_tool_model_default(&state, user_override.id)
            .await
            .expect("delete user override");
        delete_tool_model_default(&state, user_override.id)
            .await
            .expect("idempotent delete");
        let alice_views = tool_defaults_for(&state, &alice)
            .await
            .expect("alice defaults");
        let alice_view = find_view(&alice_views, "claude_code");
        assert_eq!(alice_view.scope_type, "global");
        assert_eq!(
            alice_view.default_model_client_name.as_deref(),
            Some("m-one")
        );

        // paused 发布 → 默认不可见 + warning
        update_publication(
            &state,
            publication_one.id,
            update_request(json!({"status": "paused"})),
        )
        .await
        .expect("pause publication one");
        let bob_views = tool_defaults_for(&state, &bob).await.expect("bob defaults");
        let bob_view = find_view(&bob_views, "claude_code");
        assert!(bob_view.default_model_client_name.is_none());
        assert!(
            bob_view
                .warnings
                .contains(&"default_model_not_visible".to_string())
        );
    }

    async fn seed_client_credentials(
        pool: &PgPool,
        user_id: Uuid,
        name: &str,
        allowed_models: &[&str],
    ) -> (Uuid, Uuid) {
        let device_id = Uuid::new_v4();
        let key_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO devices (id, user_id, name, platform, device_secret_hash, status) \
             VALUES ($1, $2, $3, 'macos', $4, 'active')",
        )
        .bind(device_id)
        .bind(user_id)
        .bind(name)
        .bind(format!("secret-hash-{device_id}"))
        .execute(pool)
        .await
        .expect("insert device");
        sqlx::query(
            "INSERT INTO virtual_keys \
             (id, device_id, user_id, key_prefix, key_hash, status, allowed_models) \
             VALUES ($1, $2, $3, 'sk-gw-test', $4, 'active', $5)",
        )
        .bind(key_id)
        .bind(device_id)
        .bind(user_id)
        .bind(format!("key-hash-{key_id}"))
        .bind(
            allowed_models
                .iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>(),
        )
        .execute(pool)
        .await
        .expect("insert virtual key");
        (device_id, key_id)
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_admin_catalog_preview() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let upstream_one = insert_upstream(&pool, fixture.provider_id, "m1", "c1").await;
        let upstream_two = insert_upstream(&pool, fixture.provider_id, "m2", "c1").await;
        let upstream_three = insert_upstream(&pool, fixture.provider_id, "m3", "c1").await;
        let group_one = create_model_group(
            &state,
            group_request(
                "g-one",
                "c1",
                upstream_one,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group one");
        let group_two = create_model_group(
            &state,
            group_request(
                "g-two",
                "c1",
                upstream_two,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group two");
        let group_three = create_model_group(
            &state,
            group_request(
                "g-three",
                "c1",
                upstream_three,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group three");
        let mut publication_one =
            publication_request(group_one.summary.id, json!([{"subject_type": "all_users"}]));
        publication_one.data_level_max = None;
        let publication_one = create_publication(&state, publication_one, fixture.admin_id)
            .await
            .expect("publication one");
        let mut publication_two = publication_request(
            group_two.summary.id,
            json!([{"subject_type": "all_users", "allowed": false}]),
        );
        publication_two.data_level_max = None;
        let _ = create_publication(&state, publication_two, fixture.admin_id)
            .await
            .expect("publication two");
        let mut publication_three = publication_request(
            group_three.summary.id,
            json!([{"subject_type": "user", "subject_id": fixture.alice_id}]),
        );
        publication_three.data_level_max = None;
        let _ = create_publication(&state, publication_three, fixture.admin_id)
            .await
            .expect("publication three");
        let _ = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "codex",
                "scope_type": "global",
                "default_model_group_id": group_one.summary.id,
                "allow_switch": true,
                "allowed_model_group_ids": [group_two.summary.id, group_three.summary.id]
            })),
        )
        .await
        .expect("global tool default");

        let (alice_device, alice_key) =
            seed_client_credentials(&pool, fixture.alice_id, "Alice Mac", &["g-one"]).await;
        let (bob_device, bob_key) =
            seed_client_credentials(&pool, fixture.bob_id, "Bob Mac", &[]).await;

        let before_usage: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_records")
            .fetch_one(&pool)
            .await
            .expect("usage count");
        let before_logs: i64 = sqlx::query_scalar("SELECT count(*) FROM request_logs")
            .fetch_one(&pool)
            .await
            .expect("log count");

        // 404：用户 / 设备 / Key / 归属不匹配
        let unknown_user = admin_catalog_preview(
            &state,
            CatalogPreviewQuery {
                user_id: Uuid::new_v4(),
                device_id: None,
                key_id: None,
            },
        )
        .await
        .expect_err("unknown user");
        assert!(
            matches!(
                unknown_user,
                AppError::NotFound {
                    code: "user_not_found",
                    ..
                }
            ),
            "unexpected unknown user: {unknown_user:?}"
        );
        let unknown_device = admin_catalog_preview(
            &state,
            CatalogPreviewQuery {
                user_id: fixture.alice_id,
                device_id: Some(bob_device),
                key_id: None,
            },
        )
        .await
        .expect_err("another user's device");
        assert!(
            matches!(
                unknown_device,
                AppError::NotFound {
                    code: "device_not_found",
                    ..
                }
            ),
            "unexpected device: {unknown_device:?}"
        );
        let unknown_key = admin_catalog_preview(
            &state,
            CatalogPreviewQuery {
                user_id: fixture.alice_id,
                device_id: None,
                key_id: Some(bob_key),
            },
        )
        .await
        .expect_err("another user's key");
        assert!(
            matches!(
                unknown_key,
                AppError::NotFound {
                    code: "key_not_found",
                    ..
                }
            ),
            "unexpected key: {unknown_key:?}"
        );

        // 仅 user：3 个可见模型（g2 仅可见不可用），诊断字段齐全
        let preview = admin_catalog_preview(
            &state,
            CatalogPreviewQuery {
                user_id: fixture.alice_id,
                device_id: None,
                key_id: None,
            },
        )
        .await
        .expect("preview by user");
        assert!(preview.catalog_version.starts_with("cat:"));
        assert_eq!(preview.subject.email, "alice@corp.example.com");
        assert_eq!(preview.subject.user_status, "active");
        assert!(preview.subject.key_allowed_models.is_empty());
        assert!(preview.subject.user_group_ids.is_empty());
        assert_eq!(preview.models.len(), 3);
        let model_one = preview
            .models
            .iter()
            .find(|model| model.model.id == "g-one")
            .expect("g-one visible");
        assert!(model_one.visible && model_one.allowed);
        assert_eq!(model_one.matched_by.as_deref(), Some("all_users"));
        assert_eq!(model_one.publication_id, Some(publication_one.id));
        let model_two = preview
            .models
            .iter()
            .find(|model| model.model.id == "g-two")
            .expect("g-two visible");
        assert!(model_two.visible && !model_two.allowed);
        let model_three = preview
            .models
            .iter()
            .find(|model| model.model.id == "g-three")
            .expect("g-three visible for alice");
        assert_eq!(model_three.matched_by.as_deref(), Some("user"));
        assert!(model_three.allowed);
        let codex = preview
            .tool_defaults
            .iter()
            .find(|tool| tool.tool_key == "codex")
            .expect("codex default");
        assert_eq!(codex.source, "global");
        assert_eq!(codex.default_model.as_deref(), Some("g-one"));
        assert_eq!(codex.switchable_models, vec!["g-one", "g-three"]);
        assert!(
            codex
                .warnings
                .contains(&"switchable_models_filtered".to_string())
        );
        assert_eq!(preview.enforcement.visible, "enforced");
        assert_eq!(preview.enforcement.quota, "config_only");
        assert!(
            preview
                .warnings
                .contains(&"switchable_models_filtered".to_string()),
            "{:?}",
            preview.warnings
        );
        assert!(!preview.warnings.contains(&"no_visible_models".to_string()));
        let serialized = serde_json::to_string(&preview).expect("serialize preview");
        for forbidden in ["canonical", "price", "provider", "account"] {
            assert!(
                !serialized.contains(forbidden),
                "preview leaked {forbidden}: {serialized}"
            );
        }
        assert!(serialized.contains("matched_by") && serialized.contains("publication_id"));

        // device + Key 白名单：只剩 g-one
        let preview = admin_catalog_preview(
            &state,
            CatalogPreviewQuery {
                user_id: fixture.alice_id,
                device_id: Some(alice_device),
                key_id: Some(alice_key),
            },
        )
        .await
        .expect("preview by device+key");
        assert_eq!(preview.subject.device_id, Some(alice_device));
        assert_eq!(preview.subject.device_name.as_deref(), Some("Alice Mac"));
        assert_eq!(preview.subject.key_id, Some(alice_key));
        assert_eq!(
            preview.subject.key_allowed_models,
            vec!["g-one".to_string()]
        );
        assert_eq!(preview.models.len(), 1);
        assert_eq!(preview.models[0].model.id, "g-one");
        let codex = preview
            .tool_defaults
            .iter()
            .find(|tool| tool.tool_key == "codex")
            .expect("codex default");
        assert_eq!(codex.switchable_models, vec!["g-one"]);

        // 预览不写 usage / request_logs
        let after_usage: i64 = sqlx::query_scalar("SELECT count(*) FROM usage_records")
            .fetch_one(&pool)
            .await
            .expect("usage count");
        let after_logs: i64 = sqlx::query_scalar("SELECT count(*) FROM request_logs")
            .fetch_one(&pool)
            .await
            .expect("log count");
        assert_eq!(after_usage, before_usage, "preview must not write usage");
        assert_eq!(
            after_logs, before_logs,
            "preview must not write request logs"
        );
    }

    async fn seed_user_group(pool: &PgPool, name: &str, member_ids: &[Uuid]) -> Uuid {
        let group_id = Uuid::new_v4();
        sqlx::query("INSERT INTO user_groups (id, name) VALUES ($1, $2)")
            .bind(group_id)
            .bind(name)
            .execute(pool)
            .await
            .expect("insert user group");
        for member_id in member_ids {
            sqlx::query("INSERT INTO user_group_members (group_id, user_id) VALUES ($1, $2)")
                .bind(group_id)
                .bind(member_id)
                .execute(pool)
                .await
                .expect("insert user group member");
        }
        group_id
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_user_group_publication_and_defaults() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let group_a = seed_user_group(&pool, "客服组", &[fixture.alice_id]).await;
        let group_b = seed_user_group(&pool, "研发组", &[fixture.alice_id]).await;

        let upstream_one = insert_upstream(&pool, fixture.provider_id, "m1", "c1").await;
        let upstream_two = insert_upstream(&pool, fixture.provider_id, "m2", "c1").await;
        let upstream_three = insert_upstream(&pool, fixture.provider_id, "m3", "c1").await;
        let group_one = create_model_group(
            &state,
            group_request(
                "g-one",
                "c1",
                upstream_one,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group one");
        let group_two = create_model_group(
            &state,
            group_request(
                "g-two",
                "c1",
                upstream_two,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group two");
        let group_three = create_model_group(
            &state,
            group_request(
                "g-three",
                "c1",
                upstream_three,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group three");

        let mut publication_one = publication_request(
            group_one.summary.id,
            json!([{"subject_type": "user_group", "subject_id": group_a}]),
        );
        publication_one.data_level_max = None;
        let publication_one = create_publication(&state, publication_one, fixture.admin_id)
            .await
            .expect("publication user_group");
        assert_eq!(
            publication_one.targets[0].subject_label.as_deref(),
            Some("客服组")
        );
        let mut publication_two = publication_request(
            group_two.summary.id,
            json!([{"subject_type": "user_group", "subject_id": group_b, "allowed": false}]),
        );
        publication_two.data_level_max = None;
        let _ = create_publication(&state, publication_two, fixture.admin_id)
            .await
            .expect("publication visible-only group");

        // 组内成员可见；组外（bob）不可见
        let alice = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: vec![group_a, group_b],
            key_allowed_models: Vec::new(),
        };
        let bob = CatalogSubject {
            user_id: fixture.bob_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: Vec::new(),
        };
        let alice_catalog = client_catalog(&state, &alice).await.expect("alice catalog");
        let ids: Vec<&str> = alice_catalog
            .data
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        assert_eq!(ids, vec!["g-one", "g-two"], "{ids:?}");
        assert!(
            client_catalog(&state, &bob)
                .await
                .expect("bob catalog")
                .data
                .is_empty()
        );

        // authorize / 详情
        assert_eq!(
            authorize_model(&state, &alice, "g-one")
                .await
                .expect("authorize"),
            ModelAuthorization::Allowed
        );
        assert_eq!(
            authorize_model(&state, &alice, "g-two")
                .await
                .expect("authorize"),
            ModelAuthorization::VisibleOnly
        );
        assert_eq!(
            authorize_model(&state, &alice, "g-three")
                .await
                .expect("authorize"),
            ModelAuthorization::NotPublished
        );
        assert_eq!(
            authorize_model(&state, &bob, "g-one")
                .await
                .expect("authorize"),
            ModelAuthorization::NotPublished
        );
        assert!(
            client_model_detail(&state, &alice, "g-one")
                .await
                .expect("detail")
                .is_some()
        );
        assert!(
            client_model_detail(&state, &bob, "g-one")
                .await
                .expect("detail")
                .is_none()
        );

        // 预览：subject 组实值 + matched_by=user_group
        let preview = admin_catalog_preview(
            &state,
            CatalogPreviewQuery {
                user_id: fixture.alice_id,
                device_id: None,
                key_id: None,
            },
        )
        .await
        .expect("preview alice");
        let mut group_ids = preview.subject.user_group_ids.clone();
        group_ids.sort();
        let mut expected_groups = vec![group_a, group_b];
        expected_groups.sort();
        assert_eq!(group_ids, expected_groups);
        let model_two = preview
            .models
            .iter()
            .find(|model| model.model.id == "g-two")
            .expect("g-two visible");
        assert_eq!(model_two.matched_by.as_deref(), Some("user_group"));
        assert!(!model_two.allowed);
        assert!(
            preview
                .models
                .iter()
                .all(|model| model.model.id != "g-three")
        );

        // 预留 application 发布对象仍 400；未知组 404
        let reserved = create_publication(
            &state,
            publication_request(
                group_three.summary.id,
                json!([{"subject_type": "application", "subject_id": Uuid::new_v4()}]),
            ),
            fixture.admin_id,
        )
        .await
        .expect_err("application target must be reserved");
        assert!(
            matches!(
                reserved,
                AppError::BadRequest {
                    code: "target_type_not_supported",
                    ..
                }
            ),
            "unexpected reserved: {reserved:?}"
        );
        let unknown_group = create_publication(
            &state,
            publication_request(
                group_three.summary.id,
                json!([{"subject_type": "user_group", "subject_id": Uuid::new_v4()}]),
            ),
            fixture.admin_id,
        )
        .await
        .expect_err("unknown user_group target");
        assert!(
            matches!(
                unknown_group,
                AppError::NotFound {
                    code: "user_group_not_found",
                    ..
                }
            ),
            "unexpected unknown group: {unknown_group:?}"
        );

        // 工具默认组层：global 生效于 bob；user_group 生效于 alice
        let _ = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "codex",
                "scope_type": "global",
                "default_model_group_id": group_one.summary.id
            })),
        )
        .await
        .expect("global codex default");
        let group_default = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "codex",
                "scope_type": "user_group",
                "scope_id": group_a,
                "default_model_group_id": group_one.summary.id,
                "allow_switch": true,
                "allowed_model_group_ids": [group_two.summary.id]
            })),
        )
        .await
        .expect("user_group codex default");
        assert_eq!(group_default.scope_label.as_deref(), Some("客服组"));

        let alice_views = tool_defaults_for(&state, &alice)
            .await
            .expect("alice defaults");
        let alice_codex = alice_views
            .iter()
            .find(|view| view.tool_key == "codex")
            .expect("alice codex");
        assert_eq!(alice_codex.scope_type, "user_group");
        assert_eq!(
            alice_codex.default_model_client_name.as_deref(),
            Some("g-one")
        );
        assert_eq!(
            alice_codex
                .switchable_model_groups
                .iter()
                .map(|group| group.client_model_name.as_str())
                .collect::<Vec<_>>(),
            vec!["g-one"]
        );
        let bob_views = tool_defaults_for(&state, &bob).await.expect("bob defaults");
        let bob_codex = bob_views
            .iter()
            .find(|view| view.tool_key == "codex")
            .expect("bob codex");
        assert_eq!(bob_codex.scope_type, "global");
        // Bob 不在发布对象内：global 默认仍须生效发布可见+可用，故无有效默认 + 警告。
        assert_eq!(bob_codex.default_model_client_name, None);
        assert!(
            bob_codex
                .warnings
                .iter()
                .any(|warning| warning == "default_model_not_visible"),
            "unexpected warnings: {:?}",
            bob_codex.warnings
        );

        // 多组命中确定性：后更新的 group_b 层覆盖 group_a 层（updated_at DESC）
        let _ = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "codex",
                "scope_type": "user_group",
                "scope_id": group_b,
                "default_model_group_id": group_two.summary.id
            })),
        )
        .await
        .expect("later group_b default");
        let alice_views = tool_defaults_for(&state, &alice)
            .await
            .expect("alice defaults");
        let alice_codex = alice_views
            .iter()
            .find(|view| view.tool_key == "codex")
            .expect("alice codex");
        assert_eq!(alice_codex.scope_type, "user_group");
        assert!(alice_codex.default_model_group_id.is_none());
        assert!(
            alice_codex
                .warnings
                .contains(&"default_model_not_allowed".to_string()),
            "{:?}",
            alice_codex.warnings
        );

        // 组删除后发布对象 fail-closed：label 退化 None、解析不再命中
        sqlx::query("DELETE FROM user_groups WHERE id = $1")
            .bind(group_a)
            .execute(&pool)
            .await
            .expect("delete user group");
        let page = list_publications(
            &state,
            PublicationListQuery {
                model_group_id: Some(group_one.summary.id),
                ..PublicationListQuery::default()
            },
        )
        .await
        .expect("publication list after group delete");
        assert_eq!(page.items.len(), 1);
        assert!(page.items[0].targets[0].subject_label.is_none());
        let remaining_groups = access_service::user_group_ids(&state, fixture.alice_id)
            .await
            .expect("alice groups");
        assert_eq!(remaining_groups, vec![group_b]);
        let alice = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: remaining_groups,
            key_allowed_models: Vec::new(),
        };
        assert_eq!(
            authorize_model(&state, &alice, "g-one")
                .await
                .expect("authorize"),
            ModelAuthorization::NotPublished
        );
    }

    async fn seed_provider(pool: &PgPool) -> (Uuid, Uuid) {
        let provider_id = Uuid::new_v4();
        let account_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO providers (id, name, kind, base_url) \
             VALUES ($1, $2, 'openai_compatible', 'https://p.example/v1')",
        )
        .bind(provider_id)
        .bind(format!("P-{provider_id}"))
        .execute(pool)
        .await
        .expect("insert extra provider");
        sqlx::query(
            "INSERT INTO provider_accounts (id, provider_id, name, credential_encrypted) \
             VALUES ($1, $2, $3, '\\x01'::bytea)",
        )
        .bind(account_id)
        .bind(provider_id)
        .bind(format!("acct-{account_id}"))
        .execute(pool)
        .await
        .expect("insert extra account");
        (provider_id, account_id)
    }

    async fn ungrouped_deployment(
        state: &AppState,
        upstream_model_id: Uuid,
        provider_account_id: Uuid,
    ) -> DeploymentAdmin {
        create_deployment(
            state,
            serde_json::from_value(json!({
                "upstream_model_id": upstream_model_id,
                "provider_account_id": provider_account_id
            }))
            .expect("deployment request"),
        )
        .await
        .expect("create ungrouped deployment")
    }

    async fn catalog_state_version(pool: &PgPool) -> i64 {
        sqlx::query_scalar("SELECT version FROM catalog_state WHERE id = 1")
            .fetch_one(pool)
            .await
            .expect("catalog version")
    }

    fn suggestion_list_request(canonical: &str) -> SuggestionListQuery {
        SuggestionListQuery {
            canonical: Some(canonical.to_string()),
            limit: None,
            offset: None,
        }
    }

    fn confirm_request(value: serde_json::Value) -> ConfirmMergeSuggestionRequest {
        serde_json::from_value(value).expect("confirm request")
    }

    fn reject_request(value: serde_json::Value) -> RejectMergeSuggestionRequest {
        serde_json::from_value(value).expect("reject request")
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_aggregation_suggestions() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let (provider_two, account_two) = seed_provider(&pool).await;
        let (provider_three, account_three) = seed_provider(&pool).await;

        // merge：同 canonical + 同名（跨 provider）
        let merge_a = insert_upstream(&pool, fixture.provider_id, "m-merge", "c-merge").await;
        let merge_b = insert_upstream(&pool, provider_two, "m-merge", "c-merge").await;
        let merge_account_a = fixture.account_id;
        let merge_account_b = account_two;
        // keep_separate：同 canonical + 不同名
        let keep_a = insert_upstream(&pool, fixture.provider_id, "m-keep-a", "c-keep").await;
        let keep_b = insert_upstream(&pool, provider_two, "m-keep-b", "c-keep").await;
        // stale：先两条，后追加第三条
        let stale_a = insert_upstream(&pool, fixture.provider_id, "m-stale", "c-stale").await;
        let stale_b = insert_upstream(&pool, provider_two, "m-stale", "c-stale").await;
        let stale_c = insert_upstream(&pool, provider_three, "m-stale", "c-stale").await;
        // merge_into_group：两条 + draft 目标组
        let target_a = insert_upstream(&pool, fixture.provider_id, "m-target", "c-target").await;
        let target_b = insert_upstream(&pool, provider_three, "m-target", "c-target").await;
        let target_account_b = account_three;

        let _ = ungrouped_deployment(&state, merge_a, merge_account_a).await;
        let _ = ungrouped_deployment(&state, merge_b, merge_account_b).await;
        let _ = ungrouped_deployment(&state, keep_a, fixture.account_id).await;
        let _ = ungrouped_deployment(&state, keep_b, account_two).await;
        let _ = ungrouped_deployment(&state, stale_a, fixture.account_id).await;
        let _ = ungrouped_deployment(&state, stale_b, account_two).await;
        let _ = ungrouped_deployment(&state, target_a, fixture.account_id).await;
        let _ = ungrouped_deployment(&state, target_b, target_account_b).await;
        let target_group = create_model_group(
            &state,
            serde_json::from_value(json!({
                "client_model_name": "agg-target",
                "canonical": "c-target"
            }))
            .expect("target group request"),
        )
        .await
        .expect("draft target group");

        // 列表：merge 建议、排序与字段、diff/risk_flags
        let version_before = catalog_state_version(&pool).await;
        let page = list_model_group_suggestions(
            &state,
            SuggestionListQuery {
                canonical: None,
                limit: Some(50),
                offset: None,
            },
        )
        .await
        .expect("suggestion list");
        assert_eq!(page.total, 4, "four canonical groups with >=2 deployments");
        let merge_page = list_model_group_suggestions(&state, suggestion_list_request("c-merge"))
            .await
            .expect("merge suggestions");
        assert_eq!(merge_page.items.len(), 1);
        let merge = &merge_page.items[0];
        assert_eq!(merge.recommendation, "merge");
        assert_eq!(merge.deployment_count, 2);
        assert_eq!(merge.provider_count, 2);
        assert_eq!(merge.suggestion_key.len(), 32);
        assert!(
            merge
                .deployments
                .iter()
                .all(|deployment| deployment.upstream_model == "m-merge")
        );
        assert!(
            merge.risk_flags.contains(&"multi_provider".to_string()),
            "{:?}",
            merge.risk_flags
        );
        assert!(
            merge.risk_flags.contains(&"multi_account".to_string()),
            "{:?}",
            merge.risk_flags
        );
        assert!(merge.diff.iter().any(|item| item.field == "provider"));
        assert!(merge.related_groups.is_empty());

        // confirm create_group
        let confirmation = confirm_model_group_suggestion(
            &state,
            &merge.suggestion_key,
            confirm_request(json!({
                "action": "create_group",
                "client_model_name": "agg-merge",
                "display_name": "Agg Merge"
            })),
            fixture.admin_id,
        )
        .await
        .expect("confirm create_group");
        assert_eq!(confirmation.decision, "merged");
        assert_eq!(confirmation.group.summary.client_model_name, "agg-merge");
        assert_eq!(confirmation.group.summary.status, "active");
        assert_eq!(confirmation.group.deployments.len(), 2);
        assert_eq!(confirmation.moved_deployment_ids.len(), 2);
        assert_eq!(confirmation.decided_by, Some(fixture.admin_id));
        let version_after = catalog_state_version(&pool).await;
        assert_eq!(version_after, version_before + 1);

        // 幂等：同 key 重复 confirm 返回原组
        let repeated = confirm_model_group_suggestion(
            &state,
            &merge.suggestion_key,
            confirm_request(json!({"action": "create_group", "client_model_name": "other"})),
            fixture.admin_id,
        )
        .await
        .expect("idempotent confirm");
        assert_eq!(
            repeated.group.summary.id, confirmation.group.summary.id,
            "repeated confirm must return original group"
        );
        assert!(
            list_model_group_suggestions(&state, suggestion_list_request("c-merge"))
                .await
                .expect("after merge")
                .items
                .is_empty(),
            "merged deployments disappear from suggestions"
        );

        // keep_separate：confirm 400，reject 幂等并抑制
        let keep_page = list_model_group_suggestions(&state, suggestion_list_request("c-keep"))
            .await
            .expect("keep suggestions");
        let keep = keep_page.items[0].clone();
        assert_eq!(keep.recommendation, "keep_separate");
        assert!(
            keep.risk_flags.contains(&"variant_mismatch".to_string()),
            "{:?}",
            keep.risk_flags
        );
        let not_mergeable = confirm_model_group_suggestion(
            &state,
            &keep.suggestion_key,
            confirm_request(json!({"action": "create_group", "client_model_name": "keep-one"})),
            fixture.admin_id,
        )
        .await
        .expect_err("keep_separate confirm must fail");
        assert!(
            matches!(
                not_mergeable,
                AppError::BadRequest {
                    code: "suggestion_not_mergeable",
                    ..
                }
            ),
            "unexpected: {not_mergeable:?}"
        );
        let rejection = reject_model_group_suggestion(
            &state,
            &keep.suggestion_key,
            reject_request(json!({"note": "版本不同，保持独立"})),
            fixture.admin_id,
        )
        .await
        .expect("reject");
        assert_eq!(rejection.decision, "rejected");
        assert_eq!(rejection.note, "版本不同，保持独立");
        assert_eq!(rejection.decided_by, Some(fixture.admin_id));
        assert!(
            list_model_group_suggestions(&state, suggestion_list_request("c-keep"))
                .await
                .expect("after reject")
                .items
                .is_empty(),
            "rejected suggestion must be suppressed"
        );
        let repeated_reject = reject_model_group_suggestion(
            &state,
            &keep.suggestion_key,
            reject_request(json!({"note": "again"})),
            fixture.admin_id,
        )
        .await
        .expect("idempotent reject");
        assert_eq!(repeated_reject.decision, "rejected");
        let confirm_rejected = confirm_model_group_suggestion(
            &state,
            &keep.suggestion_key,
            confirm_request(json!({"action": "create_group", "client_model_name": "nope"})),
            fixture.admin_id,
        )
        .await
        .expect_err("rejected cannot confirm");
        assert!(
            matches!(
                confirm_rejected,
                AppError::NotFound {
                    code: "suggestion_not_found",
                    ..
                }
            ),
            "unexpected: {confirm_rejected:?}"
        );

        // stale：列表后再追加部署，旧 key 409
        let stale_page = list_model_group_suggestions(&state, suggestion_list_request("c-stale"))
            .await
            .expect("stale suggestions");
        let stale = stale_page.items[0].clone();
        let _ = ungrouped_deployment(&state, stale_c, account_three).await;
        let stale_confirm = confirm_model_group_suggestion(
            &state,
            &stale.suggestion_key,
            confirm_request(json!({"action": "create_group", "client_model_name": "stale-group"})),
            fixture.admin_id,
        )
        .await
        .expect_err("stale confirm");
        assert!(
            matches!(
                stale_confirm,
                AppError::Conflict {
                    code: "suggestion_stale",
                    ..
                }
            ),
            "unexpected: {stale_confirm:?}"
        );
        let new_stale_page =
            list_model_group_suggestions(&state, suggestion_list_request("c-stale"))
                .await
                .expect("new stale suggestions");
        assert_eq!(new_stale_page.items.len(), 1);
        assert_eq!(new_stale_page.items[0].deployment_count, 3);
        assert_ne!(new_stale_page.items[0].suggestion_key, stale.suggestion_key);

        // merge_into_group
        let target_page = list_model_group_suggestions(&state, suggestion_list_request("c-target"))
            .await
            .expect("target suggestions");
        let target = target_page.items[0].clone();
        let merged_into = confirm_model_group_suggestion(
            &state,
            &target.suggestion_key,
            confirm_request(json!({
                "action": "merge_into_group",
                "target_group_id": target_group.summary.id
            })),
            fixture.admin_id,
        )
        .await
        .expect("merge_into_group");
        assert_eq!(merged_into.group.summary.id, target_group.summary.id);
        assert_eq!(merged_into.group.deployments.len(), 2);
        assert!(
            merged_into
                .group
                .deployments
                .iter()
                .all(|deployment| deployment.model_group_id == Some(target_group.summary.id))
        );

        // 未知 key / 非法 key → 404
        let unknown = confirm_model_group_suggestion(
            &state,
            &"0".repeat(32),
            confirm_request(json!({"action": "create_group", "client_model_name": "x"})),
            fixture.admin_id,
        )
        .await
        .expect_err("unknown key");
        assert!(
            matches!(
                unknown,
                AppError::NotFound {
                    code: "suggestion_not_found",
                    ..
                }
            ),
            "unexpected: {unknown:?}"
        );
        let bad_key = reject_model_group_suggestion(
            &state,
            "not-a-key",
            reject_request(json!({})),
            fixture.admin_id,
        )
        .await
        .expect_err("invalid key");
        assert!(
            matches!(
                bad_key,
                AppError::NotFound {
                    code: "suggestion_not_found",
                    ..
                }
            ),
            "unexpected: {bad_key:?}"
        );

        // 决策表落表
        let decisions: i64 = sqlx::query_scalar("SELECT count(*) FROM model_group_merge_decisions")
            .fetch_one(&pool)
            .await
            .expect("decision count");
        assert_eq!(decisions, 3, "merged + rejected decisions persisted");
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_client_model_defaults() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        let fixture = seed(&pool).await;
        let upstream_one = insert_upstream(&pool, fixture.provider_id, "m1", "c1").await;
        let upstream_two = insert_upstream(&pool, fixture.provider_id, "m2", "c1").await;
        let group_one = create_model_group(
            &state,
            group_request(
                "g-one",
                "c1",
                upstream_one,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group one");
        let group_two = create_model_group(
            &state,
            group_request(
                "g-two",
                "c1",
                upstream_two,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("group two");
        let mut publication_one =
            publication_request(group_one.summary.id, json!([{"subject_type": "all_users"}]));
        publication_one.data_level_max = None;
        let _ = create_publication(&state, publication_one, fixture.admin_id)
            .await
            .expect("publication one");
        let mut publication_two = publication_request(
            group_two.summary.id,
            json!([{"subject_type": "all_users", "allowed": false}]),
        );
        publication_two.data_level_max = None;
        let _ = create_publication(&state, publication_two, fixture.admin_id)
            .await
            .expect("publication two");
        let _ = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "codex",
                "scope_type": "global",
                "default_model_group_id": group_one.summary.id,
                "allow_switch": true,
                "allowed_model_group_ids": [group_two.summary.id]
            })),
        )
        .await
        .expect("codex default");
        // 无效默认（仅可见不可用）→ 客户端投影应省略该工具
        let _ = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "claude_code",
                "scope_type": "global",
                "default_model_group_id": group_two.summary.id
            })),
        )
        .await
        .expect("invalid claude_code default");

        let subject = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: Vec::new(),
        };
        let defaults = client_model_defaults(&state, &subject)
            .await
            .expect("client defaults");
        assert_eq!(defaults.object, "list");
        assert!(defaults.catalog_version.starts_with("cat:"));
        assert_eq!(
            defaults.data.len(),
            1,
            "invalid-default tool must be omitted"
        );
        let codex = &defaults.data[0];
        assert_eq!(codex.tool_key, "codex");
        assert_eq!(codex.default_model, "g-one");
        assert!(codex.allow_switch);
        assert_eq!(
            codex.switchable_models,
            vec!["g-one".to_string()],
            "visible-only whitelist item filtered"
        );
        let serialized = serde_json::to_string(&defaults).expect("serialize");
        for forbidden in [
            "scope_id",
            "whitelist",
            "canonical",
            "price",
            "provider",
            "account",
            "warnings",
            "allowed",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "client defaults leaked {forbidden}: {serialized}"
            );
        }

        // Key 白名单排除 → 无有效默认
        let restricted = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: vec!["other".to_string()],
        };
        assert!(
            client_model_defaults(&state, &restricted)
                .await
                .expect("restricted defaults")
                .data
                .is_empty()
        );

        // user_group 层级
        let group_id = seed_user_group(&pool, "研发组", &[fixture.alice_id]).await;
        let _ = create_tool_model_default(
            &state,
            tool_default_create(json!({
                "tool_key": "cursor",
                "scope_type": "user_group",
                "scope_id": group_id,
                "default_model_group_id": group_one.summary.id
            })),
        )
        .await
        .expect("group tool default");
        let with_group = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: vec![group_id],
            key_allowed_models: Vec::new(),
        };
        let defaults = client_model_defaults(&state, &with_group)
            .await
            .expect("group defaults");
        let cursor = defaults
            .data
            .iter()
            .find(|default| default.tool_key == "cursor")
            .expect("cursor default");
        assert_eq!(cursor.default_model, "g-one");
        assert!(!cursor.allow_switch);
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL；会 TRUNCATE catalog/providers/users 测试表"]
    async fn gated_snapshot_cache_ttl_and_invalidate() {
        let Some((state, pool)) = test_state().await else {
            return;
        };
        // 测试构建默认 TTL=0（等价关闭缓存，避免跨模块 gated 脏读）；本用例显式启用真实 TTL。
        set_snapshot_ttl_for_test(std::time::Duration::from_secs(5));
        let fixture = seed(&pool).await;
        let upstream = insert_upstream(&pool, fixture.provider_id, "m-cache", "c-cache").await;
        let group = create_model_group(
            &state,
            group_request(
                "cache-group",
                "c-cache",
                upstream,
                fixture.account_id,
                model::GROUP_STATUS_ACTIVE,
            ),
        )
        .await
        .expect("create cache group");
        let publication = create_publication(
            &state,
            publication_request(group.summary.id, json!([{"subject_type": "all_users"}])),
            fixture.admin_id,
        )
        .await
        .expect("create cache publication");
        let subject = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: Vec::new(),
        };

        // 1) 首次读：构建投影缓存；版本与投影同快照。
        let first = client_catalog(&state, &subject)
            .await
            .expect("first catalog");
        assert_eq!(first.data.len(), 1);
        let version = catalog_version(&state).await.expect("cached version");
        let db_version: i64 = sqlx::query_scalar("SELECT version FROM catalog_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("db version");
        assert_eq!(version, db_version as u64);
        assert_eq!(first.catalog_version, format!("cat:{version}"));

        // 2) 直接 SQL 暂停发布（不 bump version）：TTL 内仍命中旧快照。
        sqlx::query("UPDATE publications SET status = 'paused' WHERE id = $1")
            .bind(publication.id)
            .execute(&pool)
            .await
            .expect("direct pause");
        let cached = client_catalog(&state, &subject)
            .await
            .expect("cached catalog");
        assert_eq!(
            cached.data.len(),
            1,
            "TTL 内应命中缓存（直接 SQL 变更不可见）"
        );
        assert_eq!(
            catalog_version(&state).await.expect("cached version 2"),
            version,
            "缓存命中时版本不变"
        );

        // 3) clear 后重建：看到直接 SQL 的真值（发布已暂停 → 不可见）。
        clear_caches();
        let fresh = client_catalog(&state, &subject)
            .await
            .expect("fresh catalog");
        assert!(
            fresh.data.is_empty(),
            "clear 后应看到直接 SQL 变更（paused 发布不可见）"
        );

        // 4) 服务写后立即失效：重新激活发布（bump version），无需 clear 即可见。
        update_publication(
            &state,
            publication.id,
            update_request(json!({"status": "active"})),
        )
        .await
        .expect("reactivate publication");
        let after_write = client_catalog(&state, &subject).await.expect("after write");
        assert_eq!(after_write.data.len(), 1, "写后必须立即失效缓存");
        assert_eq!(
            after_write.catalog_version,
            format!(
                "cat:{}",
                catalog_version(&state).await.expect("post-write version")
            )
        );

        // 5) routing 视图缓存：命中/clear/写后失效。
        let views = routing_group_views(&state).await.expect("routing views");
        let initial = views
            .iter()
            .find(|view| view.group_id == group.summary.id)
            .expect("group view")
            .display_name
            .clone();
        sqlx::query("UPDATE model_groups SET display_name = 'stale-name' WHERE id = $1")
            .bind(group.summary.id)
            .execute(&pool)
            .await
            .expect("direct rename");
        let cached_views = routing_group_views(&state)
            .await
            .expect("cached routing views");
        let cached_name = cached_views
            .iter()
            .find(|view| view.group_id == group.summary.id)
            .expect("cached group view")
            .display_name
            .clone();
        assert_eq!(
            cached_name, initial,
            "TTL 内 routing 视图应命中缓存（直接 SQL 改名不可见）"
        );
        clear_caches();
        let fresh_views = routing_group_views(&state)
            .await
            .expect("fresh routing views");
        assert_eq!(
            fresh_views
                .iter()
                .find(|view| view.group_id == group.summary.id)
                .expect("fresh group view")
                .display_name,
            "stale-name"
        );
        update_model_group(
            &state,
            group.summary.id,
            serde_json::from_value(json!({"display_name": "fresh-name"})).expect("group update"),
        )
        .await
        .expect("update group display name");
        let after_write_views = routing_group_views(&state)
            .await
            .expect("routing views after write");
        assert_eq!(
            after_write_views
                .iter()
                .find(|view| view.group_id == group.summary.id)
                .expect("post-write group view")
                .display_name,
            "fresh-name",
            "写后 routing 视图缓存必须失效"
        );

        // 6) 授权复用同一投影缓存：pause（直接 SQL）+ 缓存命中仍是 Allowed；clear 后 NotPublished。
        let alice = CatalogSubject {
            user_id: fixture.alice_id,
            device_id: None,
            user_group_ids: Vec::new(),
            key_allowed_models: Vec::new(),
        };
        assert_eq!(
            authorize_model(&state, &alice, "cache-group")
                .await
                .expect("authorize"),
            ModelAuthorization::Allowed
        );
        sqlx::query("UPDATE publications SET status = 'paused' WHERE id = $1")
            .bind(publication.id)
            .execute(&pool)
            .await
            .expect("direct pause 2");
        assert_eq!(
            authorize_model(&state, &alice, "cache-group")
                .await
                .expect("cached authorize"),
            ModelAuthorization::Allowed,
            "缓存命中时授权结果与快照一致"
        );
        clear_caches();
        assert_eq!(
            authorize_model(&state, &alice, "cache-group")
                .await
                .expect("fresh authorize"),
            ModelAuthorization::NotPublished
        );
    }
}
