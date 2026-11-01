use crate::platform::http::{self, NativeHttpRequest, NativeHttpResponse};

#[tauri::command]
pub async fn native_http_request(request: NativeHttpRequest) -> Result<NativeHttpResponse, String> {
    http::request(request).await
}
