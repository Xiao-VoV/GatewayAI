use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHttpRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// 桌面 webview 受 CORS 限制，API/健康检查统一经 Rust reqwest。
pub async fn request(request: NativeHttpRequest) -> Result<NativeHttpResponse, String> {
    if !(request.url.starts_with("http://") || request.url.starts_with("https://")) {
        return Err("仅支持 http/https URL".to_string());
    }
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| "HTTP 方法不合法".to_string())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let mut builder = client.request(method, &request.url);
    for (key, value) in &request.headers {
        builder = builder.header(key, value);
    }
    if let Some(body) = &request.body {
        builder = builder.body(body.clone());
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("请求失败：{error}"))?;
    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str().to_string(), value.to_string());
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取响应失败：{error}"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("响应体超过 8MB".to_string());
    }
    Ok(NativeHttpResponse {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).to_string(),
    })
}
