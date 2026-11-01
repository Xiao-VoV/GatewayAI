import type { ReactNode } from 'react'
import { Navigate, Outlet } from 'react-router-dom'
import { useClient } from './ClientProvider'
import { ErrorPanel } from '../components/ScreenState'
import { useUiPreferences } from '../ui/preferences'

export function FullLoading({ label }: { label?: string }) {
  const { locale } = useUiPreferences()
  const text = label ?? (locale === 'en-US' ? 'Connecting to the enterprise gateway…' : '正在连接企业网关…')
  return (
    <div className="full-loading" role="status" aria-live="polite">
      <span className="spinner" aria-hidden="true" />
      <span>{text}</span>
    </div>
  )
}

export function RequireClient() {
  const client = useClient()
  const { locale } = useUiPreferences()
  const en = locale === 'en-US'
  if (client.phase === 'booting') return <FullLoading />
  if (client.phase === 'unpaired') return <Navigate to="/welcome" replace />
  if (client.phase === 'error') {
    return (
      <div className="fatal-page">
        <div className="fatal-card">
          <h1>{en ? 'Device sync failed' : '设备无法同步'}</h1>
          <ErrorPanel
            message={client.bootError ?? (en ? 'Try again, or pair this device again.' : '请重试或重新配对')}
            onRetry={() => void client.reload()}
          />
          <button className="btn" type="button" onClick={client.forget} style={{ marginTop: 12 }}>
            {en ? 'Remove pairing' : '清除本机配对'}
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
