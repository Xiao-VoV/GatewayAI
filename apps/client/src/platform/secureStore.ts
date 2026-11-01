export interface StoredCredentials {
  serverUrl: string
  deviceId: string
  deviceSecret: string
  apiKey: string
}

const WEB_SESSION_KEY = 'gw-client-session-v1'

export function isDesktopRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
}

async function invokeDesktop<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(command, args)
}

export async function saveCredentials(credentials: StoredCredentials): Promise<void> {
  if (isDesktopRuntime()) {
    await invokeDesktop('save_credentials', { payload: credentials })
    return
  }
  try {
    sessionStorage.setItem(WEB_SESSION_KEY, JSON.stringify(credentials))
  } catch (error) {
    throw new Error(`无法保存本机配对：${error instanceof Error ? error.message : 'sessionStorage 不可用'}`)
  }
}

export async function loadCredentials(): Promise<StoredCredentials | null> {
  if (isDesktopRuntime()) {
    const stored = await invokeDesktop<StoredCredentials | null>('load_credentials')
    return stored ?? null
  }
  try {
    const raw = sessionStorage.getItem(WEB_SESSION_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw) as Partial<StoredCredentials>
    if (!parsed.serverUrl || !parsed.deviceId || !parsed.deviceSecret || !parsed.apiKey) return null
    return {
      serverUrl: parsed.serverUrl,
      deviceId: parsed.deviceId,
      deviceSecret: parsed.deviceSecret,
      apiKey: parsed.apiKey,
    }
  } catch {
    return null
  }
}

export async function clearCredentials(): Promise<void> {
  if (isDesktopRuntime()) {
    await invokeDesktop('clear_credentials')
    return
  }
  try {
    sessionStorage.removeItem(WEB_SESSION_KEY)
  } catch {
    // 忽略：浏览器禁用 sessionStorage 时仅内存态。
  }
}
