import type { ReactNode } from 'react'

export function LoadingPanel({ title = '正在加载…' }: { title?: string }) {
  return (
    <div className="card" aria-busy="true" aria-live="polite">
      <div className="skeleton skeleton-line" style={{ width: 120 }} />
      <div className="skeleton skeleton-block" style={{ marginTop: 12 }} />
      <span className="hint">{title}</span>
    </div>
  )
}

export function ErrorPanel({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="statebox" role="alert">
      <h3>同步失败</h3>
      <p>{message}</p>
      <div className="actions">
        <button className="btn primary" type="button" onClick={onRetry}>
          重试
        </button>
      </div>
    </div>
  )
}

export function EmptyPanel({ title, desc, action }: { title: string; desc: string; action?: ReactNode }) {
  return (
    <div className="statebox">
      <h3>{title}</h3>
      <p>{desc}</p>
      {action && <div className="actions">{action}</div>}
    </div>
  )
}
