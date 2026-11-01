import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { ClientApiError, exchangeToken, fetchDevice, fetchModels, fetchToolDefaults, fetchUsage, pair } from '../api/clientApi'
import { clearCredentials, loadCredentials, saveCredentials } from '../platform/secureStore'
import { startLocalRouter, stopLocalRouter } from '../platform/nativeRouter'
import type { RouterInfo } from '../platform/nativeRouter'
import type { StoredCredentials } from '../platform/secureStore'
import type {
  ClientDevice,
  ClientDeviceResponse,
  ClientKey,
  ClientModel,
  ClientToolDefault,
  ClientUsage,
  ClientUser,
} from '../api/types'

const TOKEN_SKEW_MS = 60_000

export type ClientSession = StoredCredentials

export interface ClientSnapshot {
  device: ClientDevice
  user: ClientUser
  key: ClientKey
  models: ClientModel[]
  tools: ClientToolDefault[]
  usage: ClientUsage | null
  catalogVersion: string
  syncedAt: string
}

type Phase = 'booting' | 'unpaired' | 'connected' | 'error'

interface PairInput {
  serverUrl: string
  code: string
  deviceName: string
}

interface ClientContextValue {
  phase: Phase
  session: ClientSession | null
  snapshot: ClientSnapshot | null
  bootError: string | null
  syncing: boolean
  localRouter: RouterInfo | null
  routerError: string | null
  restartRouter: () => Promise<void>
  pair: (input: PairInput) => Promise<void>
  reload: () => Promise<void>
  forget: () => void
}

const ClientContext = createContext<ClientContextValue | null>(null)

export function detectPlatform(): string {
  const ua = navigator.userAgent.toLowerCase()
  if (ua.includes('mac')) return 'macos'
  if (ua.includes('win')) return 'windows'
  if (ua.includes('linux')) return 'linux'
  return 'unknown'
}

function defaultDeviceName(locale: 'zh-CN' | 'en-US' = 'zh-CN'): string {
  const platform = detectPlatform()
  if (locale === 'en-US') {
    if (platform === 'macos') return 'My Mac'
    if (platform === 'windows') return 'My Windows PC'
    if (platform === 'linux') return 'My Linux PC'
    return 'My Computer'
  }
  if (platform === 'macos') return '我的 Mac'
  if (platform === 'windows') return '我的 Windows 电脑'
  if (platform === 'linux') return '我的 Linux 电脑'
  return '我的电脑'
}

export const CLIENT_VERSION = '0.1.0'

export function ClientProvider({ children }: { children: ReactNode }) {
  const [phase, setPhase] = useState<Phase>('booting')
  const [session, setSession] = useState<ClientSession | null>(null)
  const [snapshot, setSnapshot] = useState<ClientSnapshot | null>(null)
  const [bootError, setBootError] = useState<string | null>(null)
  const [syncing, setSyncing] = useState(false)
  const [localRouter, setLocalRouter] = useState<RouterInfo | null>(null)
  const [routerError, setRouterError] = useState<string | null>(null)
  const tokenRef = useRef<{ token: string; expiresAt: number } | null>(null)

  const clear = useCallback(() => {
    tokenRef.current = null
    void clearCredentials()
    setSession(null)
    void stopLocalRouter()
    setLocalRouter(null)
    setRouterError(null)
    setSnapshot(null)
    setPhase('unpaired')
    setBootError(null)
  }, [])

  const ensureToken = useCallback(async (current: ClientSession) => {
    if (tokenRef.current && tokenRef.current.expiresAt - Date.now() > TOKEN_SKEW_MS) {
      return tokenRef.current.token
    }
    const response = await exchangeToken(current.serverUrl, current.deviceId, current.deviceSecret)
    tokenRef.current = { token: response.token, expiresAt: new Date(response.expires_at).getTime() }
    return response.token
  }, [])

  const loadSnapshot = useCallback(
    async (current: ClientSession) => {
      const token = await ensureToken(current)
      let device: ClientDeviceResponse
      try {
        device = await fetchDevice(current.serverUrl, token)
      } catch (error) {
        if (error instanceof ClientApiError && error.status === 401) {
          // token 可能刚被轮换/过期：强制换一次再试。
          tokenRef.current = null
          const fresh = await ensureToken(current)
          device = await fetchDevice(current.serverUrl, fresh)
        } else {
          throw error
        }
      }
      const authToken = tokenRef.current?.token ?? token
      const [models, tools, usage] = await Promise.all([
        fetchModels(current.serverUrl, current.apiKey, authToken),
        fetchToolDefaults(current.serverUrl, current.apiKey, authToken).catch(() => ({
          object: 'list',
          data: [],
          catalog_version: '',
        })),
        // 用量失败不阻断设备/目录；首页显示“暂不可用”。
        fetchUsage(current.serverUrl, current.apiKey, authToken).catch(() => null),
      ])
      setSnapshot({
        device: device.device,
        user: device.user,
        key: device.key,
        models: models.data,
        tools: tools.data,
        usage,
        catalogVersion: models.catalog_version ?? tools.catalog_version ?? '',
        syncedAt: new Date().toISOString(),
      })
    },
    [ensureToken],
  )

  const ensureLocalRouter = useCallback(async (current: ClientSession) => {
    try {
      const info = await startLocalRouter({
        serverUrl: current.serverUrl,
        deviceId: current.deviceId,
        deviceSecret: current.deviceSecret,
        apiKey: current.apiKey,
      })
      setLocalRouter(info)
      setRouterError(null)
    } catch (error) {
      setLocalRouter(null)
      setRouterError(error instanceof Error ? error.message : '本地路由启动失败')
    }
  }, [])

  const beginSession = useCallback(
    async (next: ClientSession) => {
      await saveCredentials(next)
      setSession(next)
      await loadSnapshot(next)
      void ensureLocalRouter(next)
      setPhase('connected')
      setBootError(null)
    },
    [loadSnapshot, ensureLocalRouter],
  )

  const bootstrap = useCallback(async () => {
    let stored: ClientSession | null
    try {
      stored = await loadCredentials()
    } catch (error) {
      setBootError(error instanceof Error ? error.message : '读取系统钥匙串失败')
      setPhase('error')
      return
    }
    if (!stored) {
      setPhase('unpaired')
      return
    }
    setSession(stored)
    try {
      await loadSnapshot(stored)
      void ensureLocalRouter(stored)
      setPhase('connected')
    } catch (error) {
      if (error instanceof ClientApiError && error.status === 401) {
        clear()
        return
      }
      if (error instanceof ClientApiError && (error.status === 403 || error.status === 0 || error.status >= 500)) {
        setBootError(error.detail)
        setPhase('error')
        return
      }
      setBootError(error instanceof Error ? error.message : '同步设备失败')
      setPhase('error')
    }
  }, [clear, loadSnapshot, ensureLocalRouter])

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  const pairDevice = useCallback(
    async (input: PairInput) => {
      const response = await pair(input.serverUrl, input.code, {
        name: input.deviceName.trim() || defaultDeviceName(),
        platform: detectPlatform(),
        client_version: CLIENT_VERSION,
      })
      tokenRef.current = {
        token: response.token,
        expiresAt: new Date(response.token_expires_at).getTime(),
      }
      const next: ClientSession = {
        serverUrl: input.serverUrl,
        deviceId: response.device.id,
        deviceSecret: response.device_secret,
        apiKey: response.api_key,
      }
      await beginSession(next)
    },
    [beginSession],
  )

  const reload = useCallback(async () => {
    if (!session) return
    setSyncing(true)
    try {
      await loadSnapshot(session)
      setBootError(null)
      setPhase('connected')
    } catch (error) {
      if (error instanceof ClientApiError && error.status === 401) {
        clear()
        return
      }
      setBootError(error instanceof ClientApiError ? error.detail : '刷新失败')
      setPhase('error')
    } finally {
      setSyncing(false)
    }
  }, [session, loadSnapshot, clear])

  const restartRouter = useCallback(async () => {
    if (!session) return
    await stopLocalRouter()
    setLocalRouter(null)
    setRouterError(null)
    await ensureLocalRouter(session)
  }, [session, ensureLocalRouter])

  const forget = useCallback(() => {
    clear()
  }, [clear])

  const value = useMemo<ClientContextValue>(
    () => ({
      phase,
      session,
      snapshot,
      bootError,
      syncing,
      localRouter,
      routerError,
      restartRouter,
      pair: pairDevice,
      reload,
      forget,
    }),
    [
      phase,
      session,
      snapshot,
      bootError,
      syncing,
      localRouter,
      routerError,
      restartRouter,
      pairDevice,
      reload,
      forget,
    ],
  )

  return <ClientContext.Provider value={value}>{children}</ClientContext.Provider>
}

export function useClient(): ClientContextValue {
  const value = useContext(ClientContext)
  if (!value) throw new Error('useClient must be used inside ClientProvider')
  return value
}

export { defaultDeviceName }
