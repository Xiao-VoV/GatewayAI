use std::sync::Arc;

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::State,
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use serde::Serialize;

use super::{RouterState, MAX_BODY_BYTES};

#[derive(Debug, Serialize)]
struct ProblemBody {
    code: &'static str,
    detail: String,
}

fn problem(status: StatusCode, code: &'static str, detail: impl Into<String>) -> Response {
    let body = ProblemBody {
        code,
        detail: detail.into(),
    };
    (status, axum::Json(body)).into_response()
}

fn hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}

/// 剥离客户端凭据/代理头；本地路由统一注入企业鉴权。
fn copy_request_headers(source: &HeaderMap) -> HeaderMap {
    let mut target = HeaderMap::new();
    for (name, value) in source {
        if hop_by_hop(name)
            || name == header::AUTHORIZATION
            || matches!(
                name.as_str(),
                "x-gw-device-token" | "x-api-key" | "x-goog-api-key"
            )
        {
            continue;
        }
        target.insert(name.clone(), value.clone());
    }
    target
}

fn copy_response_headers(source: &HeaderMap) -> HeaderMap {
    let mut target = HeaderMap::new();
    for (name, value) in source {
        if hop_by_hop(name) {
            continue;
        }
        target.insert(name.clone(), value.clone());
    }
    target
}

async fn forward(
    state: &RouterState,
    method: &Method,
    path_and_query: &str,
    headers: HeaderMap,
    body: Bytes,
    force_token: bool,
) -> Result<Response, Response> {
    let token = match state.token(force_token).await {
        Ok(token) => token,
        Err(detail) => {
            return Err(problem(
                StatusCode::BAD_GATEWAY,
                "local_token_refresh_failed",
                detail,
            ))
        }
    };
    let mut out_headers = headers;
    let auth = HeaderValue::from_str(&format!("Bearer {}", state.api_key)).map_err(|_| {
        problem(
            StatusCode::BAD_GATEWAY,
            "local_auth_invalid",
            "虚拟 Key 无法编码",
        )
    })?;
    let device = HeaderValue::from_str(&token).map_err(|_| {
        problem(
            StatusCode::BAD_GATEWAY,
            "local_token_invalid",
            "设备 token 无法编码",
        )
    })?;
    out_headers.insert(header::AUTHORIZATION, auth);
    out_headers.insert(HeaderName::from_static("x-gw-device-token"), device);

    let url = format!("{}{}", state.gateway, path_and_query);
    let method = reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|_| {
        problem(
            StatusCode::BAD_REQUEST,
            "local_method_invalid",
            "请求方法不支持",
        )
    })?;
    let response = match state
        .client
        .request(method, &url)
        .headers(out_headers)
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Err(problem(
                StatusCode::BAD_GATEWAY,
                "local_upstream_unavailable",
                format!("企业网关不可达：{error}"),
            ))
        }
    };

    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let response_headers = copy_response_headers(response.headers());
    if status == StatusCode::UNAUTHORIZED {
        return Err(problem(
            StatusCode::UNAUTHORIZED,
            "local_upstream_unauthorized",
            "企业网关鉴权失败，请检查设备/Key 状态",
        ));
    }
    let stream = response.bytes_stream().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("upstream stream: {error}"),
        )
    });
    let body = Body::from_stream(stream);
    let mut response = (status, body).into_response();
    *response.headers_mut() = response_headers;
    Ok(response)
}

pub(crate) async fn handler(
    State(state): State<Arc<RouterState>>,
    request: Request<Body>,
) -> Response {
    let (parts, body) = request.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let allowed = path_and_query == "/v1"
        || path_and_query.starts_with("/v1/")
        || path_and_query == "/v1beta"
        || path_and_query.starts_with("/v1beta/");
    if !allowed {
        return problem(
            StatusCode::NOT_FOUND,
            "local_route_not_found",
            "本地路由只代理 /v1/* 与 /v1beta/*",
        );
    }
    let body = match to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                "local_body_too_large",
                format!("请求体读取失败：{error}"),
            )
        }
    };
    match forward(
        &state,
        &parts.method,
        &path_and_query,
        copy_request_headers(&parts.headers),
        body.clone(),
        false,
    )
    .await
    {
        Ok(response) => response,
        Err(first) => {
            // 只对上游 401 做一次强制刷新重试；其它错误原样返回。
            if first.status() == StatusCode::UNAUTHORIZED {
                match forward(
                    &state,
                    &parts.method,
                    &path_and_query,
                    copy_request_headers(&parts.headers),
                    body,
                    true,
                )
                .await
                {
                    Ok(response) => response,
                    Err(second) => second,
                }
            } else {
                first
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::copy_request_headers;
    use crate::router::{start, RouterConfig};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn strips_client_auth_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer user-key"));
        headers.insert("x-gw-device-token", HeaderValue::from_static("device"));
        headers.insert("x-api-key", HeaderValue::from_static("user-key"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let copied = copy_request_headers(&headers);
        assert!(!copied.contains_key("authorization"));
        assert!(!copied.contains_key("x-gw-device-token"));
        assert!(!copied.contains_key("x-api-key"));
        assert!(copied.contains_key("content-type"));
    }

    #[tokio::test]
    async fn proxies_and_injects_credentials() {
        use axum::{extract::State, routing::get, Json, Router};
        use serde_json::json;
        use std::sync::Arc;

        #[derive(Clone, Default)]
        struct Seen {
            auth: Arc<std::sync::Mutex<Option<String>>>,
            device: Arc<std::sync::Mutex<Option<String>>>,
            query: Arc<std::sync::Mutex<Option<String>>>,
        }

        async fn token() -> Json<serde_json::Value> {
            Json(
                json!({ "token": "device-token-from-gateway", "expires_at": "2999-01-01T00:00:00Z" }),
            )
        }
        async fn beta(
            headers: axum::http::HeaderMap,
            State(seen): State<Seen>,
        ) -> (axum::http::StatusCode, [(String, String); 1], String) {
            *seen.device.lock().unwrap() = headers
                .get("x-gw-device-token")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            (
                axum::http::StatusCode::OK,
                [("content-type".to_string(), "text/event-stream".to_string())],
                "data: {}\n\n".to_string(),
            )
        }
        async fn models(
            headers: axum::http::HeaderMap,
            uri: axum::http::Uri,
            State(seen): State<Seen>,
        ) -> (axum::http::StatusCode, Json<serde_json::Value>) {
            *seen.auth.lock().unwrap() = headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            *seen.device.lock().unwrap() = headers
                .get("x-gw-device-token")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            *seen.query.lock().unwrap() = uri.query().map(str::to_string);
            (
                axum::http::StatusCode::OK,
                Json(json!({ "object": "list", "data": [] })),
            )
        }

        let seen = Seen::default();
        let gateway = Router::new()
            .route("/api/client/token", axum::routing::post(token))
            .route("/v1/models", get(models))
            .route("/v1beta/models/{model_action}", axum::routing::post(beta))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let gateway_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        tokio::spawn(async move {
            axum::serve(listener, gateway).await.unwrap();
        });

        let local = start(RouterConfig {
            server_url: gateway_url,
            device_id: "device".to_string(),
            device_secret: "secret".to_string(),
            api_key: "sk-gw-client-test".to_string(),
        })
        .await
        .expect("start local router");

        let response = reqwest::Client::new()
            .get(format!("{}/v1/models?limit=5", local.info.base_url))
            .header("authorization", "Bearer client-junk")
            .header("x-gw-device-token", "client-junk")
            .send()
            .await
            .expect("call local router");
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            seen.auth.lock().unwrap().as_deref(),
            Some("Bearer sk-gw-client-test")
        );
        assert_eq!(
            seen.device.lock().unwrap().as_deref(),
            Some("device-token-from-gateway")
        );
        assert_eq!(seen.query.lock().unwrap().as_deref(), Some("limit=5"));

        let not_found = reqwest::Client::new()
            .get(format!("{}/not-v1", local.info.base_url))
            .send()
            .await
            .expect("call local router");
        assert_eq!(not_found.status(), reqwest::StatusCode::NOT_FOUND);

        let beta_response = reqwest::Client::new()
            .post(format!(
                "{}/v1beta/models/claude-sonnet:streamGenerateContent?alt=sse",
                local.info.base_url
            ))
            .header("x-goog-api-key", "client-junk")
            .json(&serde_json::json!({"contents": []}))
            .send()
            .await
            .expect("call local router beta");
        assert_eq!(beta_response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            seen.device.lock().unwrap().as_deref(),
            Some("device-token-from-gateway")
        );
    }
}
