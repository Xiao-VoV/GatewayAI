//! policy 用例：管理台预算 CRUD + 内部预算预检。
//!
//! 预算只做判定，不写 usage；当日已用由 metering 聚合、proxy 传入。

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{Budget, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, Page, PageQuery, UpsertBudgetRequest},
    money::{BudgetDecision, CURRENCY_CNY, decide_budget, format_money, parse_daily_limit},
    repo,
};

pub fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
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

fn to_budget(row: repo::BudgetRow) -> Result<Budget, AppError> {
    let limit = Decimal::from_str_exact(&row.daily_limit)
        .map_err(|error| AppError::internal(format!("invalid budgets.daily_limit: {error}")))?;
    Ok(Budget {
        id: row.id,
        user_id: row.user_id,
        daily_limit: format_money(limit),
        currency: row.currency,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn list_budgets(state: &AppState, query: &PageQuery) -> Result<Page<Budget>, AppError> {
    let pool = db(state)?;
    let (limit, offset) = page_params(query)?;
    let (rows, total) = repo::list(pool, limit, offset).await?;
    let items = rows
        .into_iter()
        .map(to_budget)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        items,
        total,
        limit,
        offset,
    })
}

pub async fn get_budget(state: &AppState, user_id: Uuid) -> Result<Budget, AppError> {
    let pool = db(state)?;
    let row = repo::get(pool, user_id)
        .await?
        .ok_or_else(|| AppError::not_found("budget_not_found", "该用户未配置预算"))?;
    to_budget(row)
}

pub async fn upsert_budget(
    state: &AppState,
    user_id: Uuid,
    request: UpsertBudgetRequest,
) -> Result<Budget, AppError> {
    let pool = db(state)?;
    let daily_limit = parse_daily_limit(&request.daily_limit)
        .map_err(|message| AppError::validation_field("daily_limit", message))?;
    let currency = request
        .currency
        .as_deref()
        .unwrap_or(CURRENCY_CNY)
        .trim()
        .to_ascii_uppercase();
    if currency != CURRENCY_CNY {
        return Err(AppError::bad_request(
            "unsupported_currency",
            "S3 仅支持 CNY",
        ));
    }
    let row = repo::upsert(
        pool,
        Uuid::new_v4(),
        user_id,
        &format_money(daily_limit),
        &currency,
    )
    .await?;
    to_budget(row)
}

pub async fn delete_budget(state: &AppState, user_id: Uuid) -> Result<(), AppError> {
    let pool = db(state)?;
    repo::delete(pool, user_id).await?;
    Ok(())
}

/// 内部预算预检：供 S4 proxy 调用；不直接返回 HTTP。
///
/// - 无预算行 / `daily_limit=0` → Allowed；
/// - `spent_today + estimated_cost > limit` → Exceeded；
/// - DB 不可用 → 503；脏币种 → internal（失败关闭）。
#[allow(dead_code)]
pub async fn check_budget(
    state: &AppState,
    user_id: Uuid,
    spent_today: Decimal,
    estimated_cost: Decimal,
) -> Result<BudgetDecision, AppError> {
    let pool = db(state)?;
    let Some(row) = repo::get(pool, user_id).await? else {
        return Ok(BudgetDecision::Allowed);
    };
    if row.currency != CURRENCY_CNY {
        return Err(AppError::internal(format!(
            "budget currency must be {CURRENCY_CNY}, got {}",
            row.currency
        )));
    }
    let limit = Decimal::from_str_exact(&row.daily_limit)
        .map_err(|error| AppError::internal(format!("invalid budgets.daily_limit: {error}")))?;
    Ok(decide_budget(
        limit,
        spent_today,
        estimated_cost,
        CURRENCY_CNY,
        Utc::now(),
    ))
}

#[cfg(test)]
mod db_flow_tests {
    use super::*;
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
        sqlx::query("TRUNCATE users CASCADE")
            .execute(&pool)
            .await
            .expect("truncate users/budgets");
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
            crypto: Arc::new(Crypto::from_key_bytes([6u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        })
    }

    async fn insert_user(state: &AppState, id: Uuid) {
        let pool = db(state).expect("test pool");
        sqlx::query("INSERT INTO users (id, email, display_name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(format!("{id}@corp.example.com"))
            .bind("Policy Test")
            .execute(pool)
            .await
            .expect("insert user");
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE users CASCADE）"]
    async fn budget_crud_and_check() {
        let Some(state) = test_state().await else {
            return;
        };
        let user_id = Uuid::new_v4();
        insert_user(&state, user_id).await;

        let empty = list_budgets(&state, &PageQuery::default())
            .await
            .expect("list empty");
        assert_eq!(empty.total, 0);
        assert!(matches!(
            get_budget(&state, user_id).await,
            Err(AppError::NotFound {
                code: "budget_not_found",
                ..
            })
        ));

        // 无预算行 = 不限额
        assert_eq!(
            check_budget(&state, user_id, Decimal::new(999_999_999, 0), Decimal::ZERO)
                .await
                .expect("check unset"),
            BudgetDecision::Allowed
        );

        let created = upsert_budget(
            &state,
            user_id,
            UpsertBudgetRequest {
                daily_limit: "5".to_string(),
                currency: None,
            },
        )
        .await
        .expect("create budget");
        assert_eq!(created.daily_limit, "5.000000");
        assert_eq!(created.currency, "CNY");

        let fetched = get_budget(&state, user_id).await.expect("get");
        assert_eq!(fetched.id, created.id);
        let listed = list_budgets(&state, &PageQuery::default())
            .await
            .expect("list");
        assert_eq!(listed.total, 1);

        // 4.99 + 0.01 = 5 → 放行；4.99 + 0.02 = 5.01 → 超限
        assert_eq!(
            check_budget(&state, user_id, Decimal::new(499, 2), Decimal::new(1, 2))
                .await
                .expect("equal allowed"),
            BudgetDecision::Allowed
        );
        assert!(matches!(
            check_budget(
                &state,
                user_id,
                Decimal::new(499, 2),
                Decimal::new(2, 2)
            )
            .await
            .expect("exceeded"),
            BudgetDecision::Exceeded {
                retry_after_secs,
                ..
            } if retry_after_secs >= 60
        ));

        // 0 = 不限额
        let unlimited = upsert_budget(
            &state,
            user_id,
            UpsertBudgetRequest {
                daily_limit: "0.000000".to_string(),
                currency: Some("CNY".to_string()),
            },
        )
        .await
        .expect("set unlimited");
        assert_eq!(unlimited.daily_limit, "0.000000");
        assert_eq!(
            check_budget(&state, user_id, Decimal::new(999, 0), Decimal::ZERO)
                .await
                .expect("unlimited"),
            BudgetDecision::Allowed
        );

        // 非 CNY/非法金额
        assert!(matches!(
            upsert_budget(
                &state,
                user_id,
                UpsertBudgetRequest {
                    daily_limit: "1".to_string(),
                    currency: Some("USD".to_string()),
                }
            )
            .await,
            Err(AppError::BadRequest {
                code: "unsupported_currency",
                ..
            })
        ));
        assert!(matches!(
            upsert_budget(
                &state,
                user_id,
                UpsertBudgetRequest {
                    daily_limit: "-1".to_string(),
                    currency: None,
                }
            )
            .await,
            Err(AppError::Validation { .. })
        ));

        // FK 不存在用户
        assert!(matches!(
            upsert_budget(
                &state,
                Uuid::new_v4(),
                UpsertBudgetRequest {
                    daily_limit: "1".to_string(),
                    currency: None,
                }
            )
            .await,
            Err(AppError::NotFound {
                code: "user_not_found",
                ..
            })
        ));

        delete_budget(&state, user_id).await.expect("delete");
        assert!(matches!(
            get_budget(&state, user_id).await,
            Err(AppError::NotFound {
                code: "budget_not_found",
                ..
            })
        ));
        delete_budget(&state, user_id)
            .await
            .expect("delete idempotent");
    }
}
