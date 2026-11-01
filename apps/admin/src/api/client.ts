import type { Problem } from './types'

const API_BASE = (import.meta.env.VITE_API_BASE as string | undefined) ?? ''

let csrfToken: string | null = null
let unauthorizedHandler: (() => void) | null = null

export function setCsrfToken(token: string | null) {
  csrfToken = token
}

export function setUnauthorizedHandler(handler: (() => void) | null) {
  unauthorizedHandler = handler
}

export class ApiError extends Error {
  readonly status: number
  readonly code: string
  readonly detail: string
  readonly errors: Record<string, string>
  readonly requestId: string | null
  readonly retryAfterSeconds: number | null

  constructor(status: number, problem: Problem, requestId: string | null, retryAfterSeconds: number | null) {
    const detail = problem.detail ?? problem.title ?? `请求失败（HTTP ${status}）`
    super(detail)
    this.name = 'ApiError'
    this.status = status
    this.code = problem.code ?? `http_${status}`
    this.detail = detail
    this.errors = problem.errors ?? {}
    this.requestId = requestId
    this.retryAfterSeconds = retryAfterSeconds
  }
}

type RequestInitOptions = Omit<RequestInit, 'body'> & {
  body?: unknown
  /** 内部：跳过全局 401 回调（登录/初始化页面自行处理）。 */
  skipUnauthorizedHandler?: boolean
}

function parseRetryAfter(headers: Headers): number | null {
  const raw = headers.get('retry-after')
  if (!raw) return null
  const seconds = Number.parseInt(raw, 10)
  return Number.isFinite(seconds) && seconds > 0 ? seconds : null
}

async function parseJson(response: Response): Promise<unknown> {
  const text = await response.text()
  if (!text) return undefined
  try {
    return JSON.parse(text) as unknown
  } catch {
    return undefined
  }
}

export async function request<T>(path: string, options: RequestInitOptions = {}): Promise<T> {
  const { body, skipUnauthorizedHandler, headers: rawHeaders, ...rest } = options
  const headers = new Headers(rawHeaders)
  headers.set('Accept', 'application/json')
  if (body !== undefined) headers.set('Content-Type', 'application/json')

  const method = (rest.method ?? 'GET').toUpperCase()
  if (method !== 'GET' && method !== 'HEAD' && csrfToken) {
    headers.set('X-CSRF-Token', csrfToken)
  }

  let response: Response
  try {
    response = await fetch(`${API_BASE}${path}`, {
      credentials: 'include',
      ...rest,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    })
  } catch (error) {
    throw new ApiError(0, { code: 'network_error', detail: '无法连接网关，请检查服务端地址或网络' }, null, null)
  }

  const requestId = response.headers.get('x-request-id')
  const payload = await parseJson(response)

  if (!response.ok) {
    const problem =
      payload && typeof payload === 'object' && !Array.isArray(payload)
        ? (payload as Problem)
        : { title: response.statusText || '请求失败', detail: response.statusText || undefined }
    const error = new ApiError(response.status, problem, requestId, parseRetryAfter(response.headers))
    if (response.status === 401 && !skipUnauthorizedHandler) unauthorizedHandler?.()
    throw error
  }

  return payload as T
}

export function apiPath(path: string): string {
  return `${API_BASE}${path}`
}
