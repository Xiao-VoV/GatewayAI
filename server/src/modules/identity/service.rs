//! identity 用例：setup / login / me / logout。行为规则见 `技术设计.md` §3。

use chrono::{Duration, Utc};
use std::{collections::BTreeMap, time::Instant};
use uuid::Uuid;

use crate::platform::{error::AppError, state::AppState};

use super::{
    model::{
        AdminUser, ChangePasswordRequest, LoginRequest, LoginResponse, SESSION_TTL_SECS,
        SetupRequest, SetupStatus,
    },
    rate_limit, repo, security,
};

pub fn db(state: &AppState) -> Result<&sqlx::PgPool, AppError> {
    state.db.as_ref().ok_or_else(AppError::database_unavailable)
}

fn to_admin_user(row: repo::AdminRow) -> AdminUser {
    AdminUser {
        id: row.id,
        email: row.email,
        display_name: row.display_name,
        status: row.status,
        created_at: row.created_at,
    }
}

pub async fn setup_status(state: &AppState) -> Result<SetupStatus, AppError> {
    let pool = db(state)?;
    let count = repo::count_admins(pool).await?;
    Ok(SetupStatus {
        required: count == 0,
        setup_token_required: state.config.setup_token.is_some(),
    })
}

pub async fn setup(
    state: &AppState,
    request: SetupRequest,
    provided_token: Option<&str>,
    ip: &str,
) -> Result<AdminUser, AppError> {
    let mut errors = BTreeMap::new();

    let email = match security::validate_email(&request.email) {
        Ok(email) => Some(email),
        Err(message) => {
            errors.insert("email".to_string(), message);
            None
        }
    };
    if let Err(message) = security::validate_setup_password(&request.password) {
        errors.insert("password".to_string(), message);
    }
    let display_name = match security::validate_display_name(&request.display_name) {
        Ok(name) => Some(name),
        Err(message) => {
            errors.insert("display_name".to_string(), message);
            None
        }
    };
    if let Some(token) = provided_token
        && let Err(message) = security::validate_setup_token(token)
    {
        errors.insert("setup_token".to_string(), message);
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let email = email.ok_or_else(|| AppError::validation_field("email", "邮箱格式不正确"))?;
    let display_name =
        display_name.ok_or_else(|| AppError::validation_field("display_name", "显示名不合法"))?;

    // setup token：限流 → 常量时间比较（未配置时忽略）。
    if let Some(expected) = state.config.setup_token.as_deref() {
        let limiter = rate_limit::global();
        let now = Instant::now();
        if let Some(retry_after) = limiter.check_setup(ip, now) {
            return Err(AppError::too_many_attempts(retry_after));
        }
        match provided_token {
            Some(token) if security::constant_time_eq(token, expected) => limiter.clear_setup(ip),
            _ => {
                limiter.record_setup_failure(ip, now);
                return Err(AppError::setup_token_invalid());
            }
        }
    }

    let password_hash = security::hash_password(&request.password).map_err(AppError::internal)?;
    let pool = db(state)?;
    let outcome = repo::setup_admin(
        pool,
        repo::NewAdmin {
            id: Uuid::new_v4(),
            email,
            password_hash,
            display_name,
        },
    )
    .await?;

    match outcome {
        repo::SetupOutcome::Created(row) => Ok(to_admin_user(row)),
        repo::SetupOutcome::AlreadyCompleted => Err(AppError::setup_already_completed()),
        repo::SetupOutcome::Conflict => Err(AppError::setup_conflict()),
    }
}

/// 返回 `(LoginResponse, session_token)`；session_token 只用于 Set-Cookie。
pub async fn login(
    state: &AppState,
    request: LoginRequest,
    ip: &str,
    user_agent: &str,
) -> Result<(LoginResponse, String), AppError> {
    let mut errors = BTreeMap::new();
    let email = match security::validate_email(&request.email) {
        Ok(email) => Some(email),
        Err(message) => {
            errors.insert("email".to_string(), message);
            None
        }
    };
    if let Err(message) = security::validate_login_password(&request.password) {
        errors.insert("password".to_string(), message);
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let email = email.ok_or_else(|| AppError::validation_field("email", "邮箱格式不正确"))?;

    let limiter = rate_limit::global();
    let now = Instant::now();
    if let Some(retry_after) = limiter.check_login(ip, &email, now) {
        return Err(AppError::too_many_attempts(retry_after));
    }

    let pool = db(state)?;
    let Some(admin) = repo::find_admin_by_email(pool, &email).await? else {
        // 未知邮箱也做等价耗时校验，避免通过响应时间枚举账号。
        security::dummy_verify(&request.password);
        limiter.record_login_failure(ip, &email, now);
        return Err(AppError::invalid_credentials());
    };

    if !security::verify_password(&admin.password_hash, &request.password) {
        limiter.record_login_failure(ip, &email, now);
        return Err(AppError::invalid_credentials());
    }
    if admin.status != "active" {
        // 密码正确但被禁用：403，不计失败（接口设计 §4 校验顺序 6）。
        return Err(AppError::account_disabled());
    }

    let token = security::generate_token();
    let token_hash = security::sha256_hex(&token);
    let csrf_token = security::generate_token();
    let expires_at = Utc::now() + Duration::seconds(SESSION_TTL_SECS);
    repo::create_session(
        pool,
        &repo::NewSession {
            id: Uuid::new_v4(),
            user_id: admin.id,
            token_hash,
            csrf_token: csrf_token.clone(),
            user_agent: user_agent.to_string(),
            ip_address: ip.to_string(),
            expires_at,
        },
    )
    .await?;
    limiter.clear_login(ip, &email);

    let user = to_admin_user(admin);
    Ok((
        LoginResponse {
            user,
            csrf_token,
            expires_at,
        },
        token,
    ))
}

/// 修改密码（US-003）：校验当前密码、更新 Argon2id、吊销除当前会话外全部 Session。
pub async fn change_password(
    state: &AppState,
    session: &super::middleware::AdminSession,
    request: ChangePasswordRequest,
) -> Result<(), AppError> {
    let pool = db(state)?;
    let admin = repo::get_admin_by_id(pool, session.user.id)
        .await?
        .ok_or_else(AppError::not_authenticated)?;
    if admin.status != "active" {
        return Err(AppError::not_authenticated());
    }
    if !security::verify_password(&admin.password_hash, &request.current_password) {
        return Err(AppError::validation_field(
            "current_password",
            "当前密码不正确",
        ));
    }
    if let Err(message) = security::validate_setup_password(&request.new_password) {
        return Err(AppError::validation_field("new_password", message));
    }
    if request.new_password == request.current_password {
        return Err(AppError::validation_field(
            "new_password",
            "新密码不能与当前密码相同",
        ));
    }
    let password_hash = security::hash_password(&request.new_password)
        .map_err(|error| AppError::internal(format!("hash password: {error}")))?;
    repo::change_password_and_revoke_sessions(pool, admin.id, &password_hash, &session.token_hash)
        .await?;
    Ok(())
}

pub async fn logout(state: &AppState, token_hash: &str) -> Result<(), AppError> {
    let pool = db(state)?;
    let deleted = repo::delete_session(pool, token_hash).await?;
    if deleted == 0 {
        Err(AppError::not_authenticated())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod db_flow_tests {
    use super::*;
    use crate::platform::{config::Config, state::AppState};
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
        sqlx::query("TRUNCATE admin_users CASCADE")
            .execute(&pool)
            .await
            .expect("truncate identity tables");
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
            crypto: Arc::new(crate::platform::crypto::Crypto::from_key_bytes([7u8; 32])),
            http: crate::platform::http::shared_client(),
            started_at: Instant::now(),
        })
    }

    #[tokio::test]
    #[ignore = "需要 GW_TEST_DATABASE_URL 指向可清空的测试库（会 TRUNCATE admin_users）"]
    async fn setup_login_me_logout_db_flow() {
        let Some(state) = test_state().await else {
            return;
        };
        let pool = state.db.as_ref().expect("test pool");

        assert!(setup_status(&state).await.unwrap().required);
        let admin = setup(
            &state,
            SetupRequest {
                email: "Admin@Corp.Example.com".to_string(),
                password: "change-me-please-123".to_string(),
                display_name: "  平台管理员  ".to_string(),
            },
            None,
            "127.0.0.1",
        )
        .await
        .expect("setup admin");
        assert_eq!(admin.email, "admin@corp.example.com");
        assert_eq!(admin.display_name, "平台管理员");
        assert!(!setup_status(&state).await.unwrap().required);

        let duplicate = setup(
            &state,
            SetupRequest {
                email: "other@corp.example.com".to_string(),
                password: "change-me-please-123".to_string(),
                display_name: String::new(),
            },
            None,
            "127.0.0.1",
        )
        .await
        .expect_err("second setup must be rejected");
        assert!(matches!(
            duplicate,
            AppError::Forbidden {
                code: "setup_already_completed",
                ..
            }
        ));

        let wrong = login(
            &state,
            LoginRequest {
                email: "admin@corp.example.com".to_string(),
                password: "wrong-password".to_string(),
            },
            "127.0.0.1",
            "test-agent",
        )
        .await
        .expect_err("wrong password must fail");
        assert!(matches!(
            wrong,
            AppError::Unauthorized {
                code: "invalid_credentials",
                ..
            }
        ));

        let (response, token) = login(
            &state,
            LoginRequest {
                email: "admin@corp.example.com".to_string(),
                password: "change-me-please-123".to_string(),
            },
            "127.0.0.1",
            "test-agent",
        )
        .await
        .expect("login");
        assert_eq!(response.user.email, "admin@corp.example.com");
        assert_eq!(response.csrf_token.len(), 43);

        let token_hash = security::sha256_hex(&token);
        let session = repo::find_session_user(pool, &token_hash)
            .await
            .unwrap()
            .expect("session row must exist");
        assert_eq!(session.csrf_token, response.csrf_token);
        assert_eq!(session.status, "active");

        logout(&state, &token_hash).await.expect("logout");
        assert!(
            repo::find_session_user(pool, &token_hash)
                .await
                .unwrap()
                .is_none()
        );
    }
}
