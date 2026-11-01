import type { ReactNode } from 'react'
import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { useI18n } from '../i18n/I18nProvider'
import { useAuth } from './AuthProvider'

export function FullLoading({ label }: { label?: string }) {
  const { tr } = useI18n()
  return (
    <div className="full-loading" role="status" aria-live="polite">
      <span className="spinner" aria-hidden="true" />
      <span>{label ?? tr("connecting_to_the_gateway")}</span>
    </div>
  )
}

/** 受保护布局：未登录跳登录并带 from；空库先初始化。 */
export function RequireAuth() {
  const auth = useAuth()
  const location = useLocation()
  const { tr } = useI18n()

  if (auth.status === 'booting') return <FullLoading />
  if (auth.status === 'error') {
    return (
      <div className="login-wrap">
        <div className="login-card">
          <h1>{tr("cannot_reach_the_admin_service")}</h1>
          <p>{auth.bootError ?? tr("make_sure_the_gateway_is_running_then_try_again")}</p>
          <button className="btn primary" type="button" onClick={auth.retryBootstrap}>
            {tr("retry")}
          </button>
        </div>
      </div>
    )
  }
  if (auth.setupRequired) return <Navigate to="/setup" replace />
  if (!auth.session) {
    const from = `${location.pathname}${location.search}`
    return <Navigate to={`/login?from=${encodeURIComponent(from)}`} replace />
  }
  return <Outlet />
}

/** /setup：仅空库可访问；已完成初始化跳登录/总览。 */
export function SetupGate({ children }: { children: ReactNode }) {
  const auth = useAuth()
  const { tr } = useI18n()
  if (auth.status === 'booting') return <FullLoading label={tr("checking_setup_status")} />
  if (auth.status === 'error') return <Navigate to="/login" replace />
  if (auth.session) return <Navigate to="/" replace />
  if (!auth.setupRequired) return <Navigate to="/login" replace />
  return <>{children}</>
}

/** /login：已登录跳总览；空库跳初始化。 */
export function LoginGate({ children }: { children: ReactNode }) {
  const auth = useAuth()
  if (auth.status === 'booting') return <FullLoading />
  if (auth.status === 'error') return <Navigate to="/login" replace />
  if (auth.session) return <Navigate to="/" replace />
  if (auth.setupRequired) return <Navigate to="/setup" replace />
  return <>{children}</>
}
