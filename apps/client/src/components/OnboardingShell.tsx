import type { ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { CLIENT_VERSION } from '../auth/ClientProvider'
import { useUiPreferences } from '../ui/preferences'

export interface OnboardingPoint {
  title: string
  desc: string
}

export function OnboardingShell({
  eyebrow,
  title,
  description,
  points,
  children,
}: {
  eyebrow?: string
  title: string
  description: string
  points: OnboardingPoint[]
  children: ReactNode
}) {
  const { locale } = useUiPreferences()

  return (
    <div className="onboarding">
      <aside className="onboarding-side">
        <Link className="onboarding-brand" to="/welcome">
          <img className="brand-mark-img" src="/mark-cream.svg" alt="Orveil" />
          <span>
            Orveil
            <small>{locale === 'en-US' ? 'AI Gateway' : '帷幄 · 企业 AI 网关'}</small>
          </span>
        </Link>
        <div className="onboarding-copy">
          {eyebrow && <span className="eyebrow">{eyebrow}</span>}
          <h1>{title}</h1>
          <p>{description}</p>
        </div>
        <ul className="onboarding-points">
          {points.map((point) => (
            <li key={point.title}>
              <strong>{point.title}</strong>
              <span>{point.desc}</span>
            </li>
          ))}
        </ul>
        <div className="onboarding-side-foot">
          <span>{locale === 'en-US' ? 'Device bound' : '设备绑定'}</span>
          <span>{locale === 'en-US' ? 'Short-lived tokens' : '短期凭证'}</span>
          <span>v{CLIENT_VERSION}</span>
        </div>
      </aside>
      <section className="onboarding-main">
        <div className="onboarding-body">{children}</div>
      </section>
    </div>
  )
}
