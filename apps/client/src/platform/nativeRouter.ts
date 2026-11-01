import { isDesktopRuntime } from './secureStore'

export interface RouterInfo {
  port: number
  baseUrl: string
}

export interface RouterSession {
  serverUrl: string
  deviceId: string
  deviceSecret: string
  apiKey: string
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const api = await import('@tauri-apps/api/core')
  return api.invoke<T>(command, args)
}

export async function startLocalRouter(session: RouterSession): Promise<RouterInfo> {
  if (!isDesktopRuntime()) throw new Error('本地路由需要 Tauri 桌面版')
  return invoke<RouterInfo>('start_local_router', {
    config: {
      serverUrl: session.serverUrl,
      deviceId: session.deviceId,
      deviceSecret: session.deviceSecret,
      apiKey: session.apiKey,
    },
  })
}

export async function stopLocalRouter(): Promise<void> {
  if (!isDesktopRuntime()) return
  await invoke('stop_local_router')
}

export async function localRouterStatus(): Promise<{ running: boolean; port: number | null; baseUrl: string | null }> {
  if (!isDesktopRuntime()) return { running: false, port: null, baseUrl: null }
  return invoke('local_router_status')
}
