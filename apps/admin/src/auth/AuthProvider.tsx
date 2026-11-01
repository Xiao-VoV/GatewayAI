import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { setCsrfToken, setUnauthorizedHandler, ApiError } from '../api/client'
import { getSetupStatus, login as apiLogin, logout as apiLogout, me, setup as apiSetup } from '../api/auth'
import type { AdminUser, LoginResponse } from '../api/types'
import { useI18n } from '../i18n/I18nProvider'
import { setApiLocale } from '../api/client'

type BootStatus = 'booting' | 'ready' | 'error'

interface AuthContextValue {
  status: BootStatus
  bootError: string | null
  setupRequired: boolean
  setupTokenRequired: boolean
  session: LoginResponse | null
  retryBootstrap: () => void
  login: (email: string, password: string) => Promise<void>
  setupAdmin: (input: { email: string; password: string; displayName?: string; setupToken?: string }) => Promise<AdminUser>
  logout: () => Promise<void>
}

const AuthContext = createContext<AuthContextValue | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const { tr, locale } = useI18n()
  useEffect(() => setApiLocale(locale), [locale])
  const [status, setStatus] = useState<BootStatus>('booting')
  const [bootError, setBootError] = useState<string | null>(null)
  const [setupRequired, setSetupRequired] = useState(false)
  const [setupTokenRequired, setSetupTokenRequired] = useState(false)
  const [session, setSession] = useState<LoginResponse | null>(null)

  const clearSession = useCallback(() => {
    setCsrfToken(null)
    setSession(null)
  }, [])

  const bootstrap = useCallback(async () => {
    setStatus('booting')
    setBootError(null)
    try {
      const statusResponse = await getSetupStatus()
      setSetupRequired(statusResponse.required)
      setSetupTokenRequired(statusResponse.setup_token_required)
      if (statusResponse.required) {
        clearSession()
        setStatus('ready')
        return
      }
      try {
        const sessionResponse = await me()
        setCsrfToken(sessionResponse.csrf_token)
        setSession(sessionResponse)
        setStatus('ready')
      } catch (error) {
        if (error instanceof ApiError && error.status === 401) {
          clearSession()
          setStatus('ready')
          return
        }
        throw error
      }
    } catch (error) {
      const message = error instanceof ApiError ? error.detail : tr('无法连接管理服务', 'Unable to reach the admin service')
      setBootError(message)
      setStatus('error')
    }
  }, [clearSession, tr])

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  useEffect(() => {
    setUnauthorizedHandler(() => {
      clearSession()
    })
    return () => setUnauthorizedHandler(null)
  }, [clearSession])

  const login = useCallback(async (email: string, password: string) => {
    const response = await apiLogin({ email, password })
    setCsrfToken(response.csrf_token)
    setSession(response)
    setSetupRequired(false)
    setStatus('ready')
  }, [])

  const setupAdmin = useCallback(
    async (input: { email: string; password: string; displayName?: string; setupToken?: string }) => {
      const user = await apiSetup({
        email: input.email,
        password: input.password,
        display_name: input.displayName,
        setupToken: input.setupToken,
      })
      setSetupRequired(false)
      setSetupTokenRequired(false)
      return user
    },
    [],
  )

  const logout = useCallback(async () => {
    try {
      await apiLogout()
    } catch {
      // 会话可能已过期；本地清理优先。
    } finally {
      clearSession()
      setStatus('ready')
    }
  }, [clearSession])

  const value = useMemo<AuthContextValue>(
    () => ({
      status,
      bootError,
      setupRequired,
      setupTokenRequired,
      session,
      retryBootstrap: () => void bootstrap(),
      login,
      setupAdmin,
      logout,
    }),
    [status, bootError, setupRequired, setupTokenRequired, session, bootstrap, login, setupAdmin, logout],
  )

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export function useAuth(): AuthContextValue {
  const value = useContext(AuthContext)
  if (!value) throw new Error('useAuth must be used inside AuthProvider')
  return value
}
