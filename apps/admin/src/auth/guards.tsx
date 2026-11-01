import type { ReactNode } from 'react'
import { Navigate, Outlet, useLocation } from 'react-router-dom'
import { useAuth } from './AuthProvider'

export function FullLoading({ label = '正在连接网关…' }: { label?: string }) {
  return (
    <div className="full-loading" role="status" aria-live="polite">
      <span className="spinner" aria-hidden="true" />
      <span>{label}</span>
    </div>
  )
}

/** 受保护布局：未登录跳登录并带 from；空库先初始化。 */
export function RequireAuth() {
  const auth = useAuth()
  const location = useLocation()

  if (auth.status === 'booting') return <FullLoading />
  if (auth.status === 'error') {
    return (
      <div className="login-wrap">
        <div className="login-card">
          <h1>无法连接管理服务</h1>
          <p>{auth.bootError ?? '请确认网关服务已启动后重试。'}</p>
          <button className="btn primary" type="button" onClick={auth.retryBootstrap}>
            重试
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
  if (auth.status === 'booting') return <FullLoading label="正在检查初始化状态…" />
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
