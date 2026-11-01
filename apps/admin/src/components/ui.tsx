import type { ReactNode } from 'react'

export function Badge({
  children,
  tone = '',
}: {
  children: ReactNode
  tone?: '' | 'ok' | 'warn' | 'bad' | 'accent'
}) {
  return <span className={`badge ${tone}`}>{children}</span>
}

export function PageHead({ title, desc, actions }: { title: string; desc?: string; actions?: ReactNode }) {
  return (
    <div className="page-head">
      <div>
        <h2>{title}</h2>
        {desc && <p>{desc}</p>}
      </div>
      {actions && <div className="actions">{actions}</div>}
    </div>
  )
}

export function StatusRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="status-row">
      <span>{label}</span>
      <span style={{ marginLeft: 'auto' }}>{children}</span>
    </div>
  )
}

export function Skeleton({ width = '100%', height = 12 }: { width?: number | string; height?: number }) {
  return <div className="skeleton skeleton-line" style={{ width, height }} />
}
