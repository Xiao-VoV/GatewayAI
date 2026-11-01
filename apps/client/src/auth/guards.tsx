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
      <div className="pairing-wrap">
        <div className="pairing-card pairing-body">
          <h2 style={{ marginTop: 0, fontSize: 17 }}>设备无法同步</h2>
          <ErrorPanel message={client.bootError ?? '请重试或重新配对'} onRetry={() => void client.reload()} />
          <button className="btn" type="button" onClick={client.forget} style={{ marginTop: 10 }}>
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
