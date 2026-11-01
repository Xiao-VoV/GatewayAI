import { isDesktopRuntime } from './secureStore'

export interface NativeHttpResponse {
  status: number
  headers: Record<string, string>
  body: string
}

/** 桌面 WebView 受 CORS 限制；全部 HTTP 走 Rust reqwest。浏览器预览仍用 fetch。 */
export async function nativeHttpRequest(
  method: string,
  url: string,
  headers: Record<string, string>,
  body: string | null,
): Promise<NativeHttpResponse> {
  if (!isDesktopRuntime()) throw new Error('native HTTP 仅在桌面版可用')
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<NativeHttpResponse>('native_http_request', {
    request: { method, url, headers, body },
  })
}
