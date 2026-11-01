import type { ReactNode } from 'react'
import { Navigate, Outlet } from 'react-router-dom'
import { useClient } from './ClientProvider'
import { ErrorPanel } from '../components/ScreenState'

export function FullLoading({ label = '正在连接企业网关…' }: { label?: string }) {
  return (
    <div className="full-loading" role="status" aria-live="polite">
      <span className="spinner" aria-hidden="true" />
      <span>{label}</span>
    </div>
  )
}

export function RequireClient() {
  const client = useClient()
  if (client.phase === 'booting') return <FullLoading />
  if (client.phase === 'unpaired') return <Navigate to="/welcome" replace />
  if (client.phase === 'error') {
    return (
      <div className="fatal-page">
        <div className="fatal-card">
          <h1>设备无法同步</h1>
          <ErrorPanel message={client.bootError ?? '请重试或重新配对'} onRetry={() => void client.reload()} />
          <button className="btn" type="button" onClick={client.forget} style={{ marginTop: 12 }}>
            清除本机配对
          </button>
        </div>
      </div>
    )
  }
  return <Outlet />
}

export function UnpairedOnly({ children }: { children: ReactNode }) {
  const client = useClient()
  if (client.phase === 'booting') return <FullLoading />
  if (client.phase === 'connected') return <Navigate to="/" replace />
  return <>{children}</>
}
