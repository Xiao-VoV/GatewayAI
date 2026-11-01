import { useState } from 'react'
import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { CLIENT_VERSION, useClient } from '../auth/ClientProvider'
import { useUiPreferences } from '../ui/preferences'
import { Icon } from './icons'

const nav = [
  { to: '/', labelKey: 'nav.home', icon: 'home' as const, end: true },
  { to: '/tools', labelKey: 'nav.tools', icon: 'tools' as const },
  { to: '/routing', labelKey: 'nav.routing', icon: 'routing' as const },
  { to: '/models', labelKey: 'nav.models', icon: 'models' as const },
  { to: '/devices', labelKey: 'nav.devices', icon: 'devices' as const },
  { to: '/settings', labelKey: 'nav.settings', icon: 'settings' as const },
]

const titleKeys: Record<string, string> = {
  '/': 'nav.home',
  '/tools': 'nav.tools',
  '/routing': 'nav.routing',
  '/models': 'nav.models',
  '/devices': 'nav.devices',
  '/settings': 'nav.settings',
  '/recovery': 'nav.recovery',
}

function initials(value: string) {
  return value.trim().slice(0, 1).toUpperCase() || 'G'
}

export function ClientShell() {
  const client = useClient()
  const { pathname } = useLocation()
  const { t, locale, resolvedTheme, setTheme } = useUiPreferences()
  const [refreshing, setRefreshing] = useState(false)

  async function handleRefresh() {
    setRefreshing(true)
    try {
      await client.reload()
    } finally {
      setRefreshing(false)
    }
  }

  const title = t(titleKeys[pathname] ?? 'nav.home')
  const user = client.snapshot?.user
  const displayName = user?.display_name || user?.email?.split('@')[0] || 'Employee'

  return (
    <div className="client-app">
      <aside className="client-sidebar">
        <div className="client-brand">
          <img className="brand-mark-img" src={resolvedTheme === 'dark' ? '/mark-cream.svg' : '/mark-ink.svg'} alt="Orveil" />
          <span>
            Orveil
            <small>{locale === 'zh-CN' ? '帷幄 · 企业 AI 网关' : 'AI Gateway'}</small>
          </span>
        </div>

        <nav className="client-nav">
          <span className="client-nav-label">{t('app.workspace')}</span>
          {nav.map((item) => (
            <NavLink key={item.to} to={item.to} end={item.end} className={({ isActive }) => (isActive ? 'active' : '')}>
              <Icon name={item.icon} className="nav-icon" />
              <span>{t(item.labelKey)}</span>
            </NavLink>
          ))}
        </nav>

        <div className="client-sidebar-foot">
          <div className="device-chip">
            <span className={`device-dot ${client.phase === 'connected' ? '' : 'off'}`} />
            <span className="device-text">
              <strong>{client.snapshot?.device.name ?? '—'}</strong>
              <small>{client.snapshot?.device.platform ?? '—'}</small>
            </span>
          </div>
          <div className="sidebar-meta">
            <span>v{CLIENT_VERSION}</span>
            <span>{client.snapshot?.device.id.slice(0, 8) ?? '—'}</span>
          </div>
        </div>
      </aside>

      <div className="client-main">
        <header className="client-topbar">
          <div className="client-title">
            <div className="client-breadcrumb">
              <span>{t('app.workspace')}</span>
              <i>/</i>
              <strong>{title}</strong>
            </div>
            <span>{client.session?.serverUrl ?? '—'}</span>
          </div>
          <div className="client-topbar-actions">
            <span className={`connection-pill ${client.phase === 'connected' ? 'ok' : 'off'}`}>
              <i />
              {client.phase === 'connected' ? t('status.connected') : t('status.disconnected')}
            </span>
            <button
              className="icon-btn"
              type="button"
              title={t('theme.toggle')}
              aria-label={t('theme.toggle')}
              onClick={() => setTheme(resolvedTheme === 'dark' ? 'light' : 'dark')}
            >
              <Icon name={resolvedTheme === 'dark' ? 'sun' : 'moon'} />
            </button>
            <button className="btn sm" type="button" onClick={() => void handleRefresh()} disabled={refreshing}>
              {refreshing ? t('action.syncing') : t('action.sync')}
            </button>
            <span className="topbar-divider" />
            <span className="topbar-user">
              <span className="avatar">{initials(displayName)}</span>
              <span>
                <strong>{displayName}</strong>
                <small>{user?.email ?? ''}</small>
              </span>
            </span>
          </div>
        </header>

        <main className="client-content">
          <Outlet />
        </main>
      </div>
    </div>
  )
}
