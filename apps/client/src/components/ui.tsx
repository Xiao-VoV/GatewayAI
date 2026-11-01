import type { ReactNode } from 'react'

export function Badge({ children, tone = '' }: { children: ReactNode; tone?: '' | 'ok' | 'warn' | 'bad' | 'accent' }) {
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

export function Stat({ label, children, hint }: { label: string; children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="card stat">
      <div className="sub">{label}</div>
      <div className="num">{children}</div>
      {hint && <div className="delta">{hint}</div>}
    </div>
  )
}
