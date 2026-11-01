import type { ReactNode } from 'react'
import type { ApiError } from '../api/client'
import { useI18n } from '../i18n/I18nProvider'

export function LoadingPanel({ title }: { title?: string }) {
  const { tr } = useI18n()
  const label = title ?? tr('正在加载…', 'Loading…')
  return (
    <div className="card" aria-busy="true" aria-live="polite">
      <div className="skeleton skeleton-line" style={{ width: 120 }} />
      <div className="skeleton skeleton-block" style={{ marginTop: 12 }} />
      <div className="skeleton skeleton-block" style={{ marginTop: 10 }} />
      <span className="hint">{label}</span>
    </div>
  )
}

export function ErrorPanel({ error, onRetry }: { error: ApiError; onRetry: () => void }) {
  const { tr } = useI18n()
  return (
    <div className="statebox" role="alert">
      <h3>{tr('加载失败', 'Failed to load')}</h3>
      <p>{error.detail}</p>
      <p className="hint">
        {error.code}
        {error.requestId ? ` · request-id ${error.requestId}` : ''}
      </p>
      <div className="actions">
        <button className="btn primary" type="button" onClick={onRetry}>
          {tr('重试', 'Retry')}
        </button>
      </div>
    </div>
  )
}

export function EmptyPanel({
  title,
  desc,
  action,
}: {
  title: string
  desc: string
  action?: ReactNode
}) {
  return (
    <div className="statebox">
      <h3>{title}</h3>
      <p>{desc}</p>
      {action && <div className="actions">{action}</div>}
    </div>
  )
}
