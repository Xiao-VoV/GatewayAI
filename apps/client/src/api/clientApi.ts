import { isDesktopRuntime } from '../platform/secureStore'
import { nativeHttpRequest } from '../platform/nativeHttp'
import type {
  ClientDeviceResponse,
  ClientModelList,
  ClientToolDefaults,
  ClientUsage,
  PairResponse,
  TokenResponse,
} from './types'

export class ClientApiError extends Error {
  readonly status: number
  readonly code: string
  readonly detail: string
  readonly errors: Record<string, string>
  readonly retryAfterSeconds: number | null

  constructor(status: number, code: string, detail: string, errors: Record<string, string> = {}, retryAfter: number | null = null) {
    super(detail)
    this.name = 'ClientApiError'
    this.status = status
    this.code = code
    this.detail = detail
    this.errors = errors
    this.retryAfterSeconds = retryAfter
  }
}

export function normalizeServerUrl(raw: string): string {
  return raw.trim().replace(/\/+$/, '')
}

function retryAfter(headers: Headers): number | null {
  const value = headers.get('retry-after')
  if (!value) return null
  const seconds = Number.parseInt(value, 10)
  return Number.isFinite(seconds) && seconds > 0 ? seconds : null
}

interface RequestOptions {
  method?: 'GET' | 'POST'
  token?: string
  apiKey?: string
  deviceToken?: string
  body?: unknown
}

async function requestJson<T>(serverUrl: string, path: string, options: RequestOptions = {}): Promise<T> {
  const headers = new Headers({ Accept: 'application/json' })
  if (options.body !== undefined) headers.set('Content-Type', 'application/json')
  if (options.token) headers.set('Authorization', `Bearer ${options.token}`)
  if (options.apiKey) headers.set('Authorization', options.apiKey)
  if (options.deviceToken) headers.set('X-GW-Device-Token', options.deviceToken)

  const method = options.method ?? 'GET'
  const bodyText = options.body === undefined ? null : JSON.stringify(options.body)
  let status: number
  let responseHeaders: Headers
  let text: string
  try {
    if (isDesktopRuntime()) {
      const response = await nativeHttpRequest(
        method,
        `${normalizeServerUrl(serverUrl)}${path}`,
        Object.fromEntries(headers.entries()),
        bodyText,
      )
      status = response.status
      responseHeaders = new Headers(response.headers)
      text = response.body
    } else {
      const response = await fetch(`${normalizeServerUrl(serverUrl)}${path}`, {
        method,
        headers,
        body: bodyText,
      })
      status = response.status
      responseHeaders = response.headers
      text = await response.text()
    }
  } catch {
    throw new ClientApiError(0, 'network_error', '无法连接企业网关，请检查地址与网络')
  }

  let payload: unknown = undefined
  if (text) {
    try {
      payload = JSON.parse(text) as unknown
    } catch {
      payload = undefined
    }
  }

  if (status < 200 || status >= 300) {
    const problem =
      payload && typeof payload === 'object' && !Array.isArray(payload)
        ? (payload as { code?: string; detail?: string; title?: string; errors?: Record<string, string> })
        : {}
    throw new ClientApiError(
      status,
      problem.code ?? `http_${status}`,
      problem.detail ?? problem.title ?? `请求失败（HTTP ${status}）`,
      problem.errors ?? {},
      retryAfter(responseHeaders),
    )
  }
  return payload as T
}

export function pair(serverUrl: string, code: string, device: { name: string; platform: string; client_version: string }) {
  return requestJson<PairResponse>(serverUrl, '/api/client/pair', {
    method: 'POST',
    body: { code, device },
  })
}

export function exchangeToken(serverUrl: string, deviceId: string, deviceSecret: string) {
  return requestJson<TokenResponse>(serverUrl, '/api/client/token', {
    method: 'POST',
    body: { device_id: deviceId, device_secret: deviceSecret },
  })
}

export function fetchDevice(serverUrl: string, token: string) {
  return requestJson<ClientDeviceResponse>(serverUrl, '/api/client/device', { token })
}

export function fetchModels(serverUrl: string, apiKey: string, deviceToken: string) {
  return requestJson<ClientModelList>(serverUrl, '/v1/models', { apiKey, deviceToken })
}

export function fetchToolDefaults(serverUrl: string, apiKey: string, deviceToken: string) {
  return requestJson<ClientToolDefaults>(serverUrl, '/v1/model-defaults', { apiKey, deviceToken })
}

export function fetchUsage(serverUrl: string, apiKey: string, deviceToken: string, days = 7) {
  return requestJson<ClientUsage>(serverUrl, `/api/client/usage?days=${encodeURIComponent(days)}`, {
    apiKey,
    deviceToken,
  })
}
