//! settings HTTP：`GET/PATCH /api/admin/settings`（AdminSession；PATCH 需 CSRF；no-store）。

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::modules::identity::middleware::{AdminSession, require_csrf};
use crate::platform::{error::AppError, state::AppState};

use super::{model::PatchSettingsRequest, service};

const MAX_BODY_BYTES: usize = 64 * 1024;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/settings",
            get(get_settings).patch(patch_settings),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

async fn get_settings(
    State(state): State<AppState>,
    _session: AdminSession,
) -> Result<Response, AppError> {
    let settings = service::get_settings(&state).await?;
    Ok(no_store_json(settings))
}

async fn patch_settings(
    State(state): State<AppState>,
    session: AdminSession,
    headers: HeaderMap,
    payload: Result<Json<PatchSettingsRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    require_csrf(&session, &headers)?;
    let request = match payload {
        Ok(Json(request)) => request,
        Err(rejection) => {
            return Err(AppError::validation_field(
                "body",
                format!("请求体不合法: {}", rejection.body_text()),
            ));
        }
    };
    let settings = service::patch_settings(&state, request).await?;
    Ok(no_store_json(settings))
}

fn no_store_json<T: serde::Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
