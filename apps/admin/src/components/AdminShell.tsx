import { useState } from 'react'
import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { useAuth } from '../auth/AuthProvider'
import { LocaleSwitch } from '../i18n/LocaleSwitch'
import { useI18n } from '../i18n/I18nProvider'

function initials(name: string, fallback: string) {
  const value = name.trim()
  if (!value) return fallback
  return value.slice(0, 1).toUpperCase()
}

export function AdminShell() {
  const { session, logout } = useAuth()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const { tr } = useI18n()
  const [loggingOut, setLoggingOut] = useState(false)

  const groups = [
    {
      title: tr("operations"),
      items: [
        { to: '/', label: tr("overview"), end: true },
        { to: '/logs', label: tr("request_logs") },
        { to: '/audit', label: tr("audit") },
      ],
    },
    {
      title: tr("resources"),
      items: [
        { to: '/providers', label: tr("providers") },
        { to: '/models', label: tr("model_catalog") },
        { to: '/tool-defaults', label: tr("tool_defaults") },
        { to: '/catalog-preview', label: tr("employee_catalog") },
        { to: '/keys', label: tr("virtual_keys") },
        { to: '/devices', label: tr("devices_pairing") },
        { to: '/groups', label: tr("users_groups") },
      ],
    },
    {
      title: tr("system"),
      items: [
        { to: '/settings', label: tr("settings") },
        { to: '/account', label: tr("account_session") },
      ],
    },
  ]

  const titles: Record<string, string> = {
    '/': tr("overview"),
    '/providers': tr("providers"),
    '/models': tr("model_catalog"),
    '/tool-defaults': tr("tool_defaults"),
    '/catalog-preview': tr("employee_catalog"),
    '/keys': tr("virtual_keys"),
    '/devices': tr("devices_pairing"),
    '/groups': tr("users_groups"),
    '/logs': tr("request_logs"),
    '/audit': tr("audit"),
    '/settings': tr("settings"),
    '/account': tr("account_session"),
  }

  function titleFor(path: string) {
    if (path.startsWith('/providers/')) return tr("provider_detail")
    if (path.startsWith('/models/')) return tr("model_group_detail")
    return titles[path] ?? tr("console")
  }

  const user = session?.user
  const displayName = user?.display_name || user?.email || tr("administrator")

  async function handleLogout() {
    setLoggingOut(true)
    try {
      await logout()
      navigate('/login', { replace: true })
    } finally {
      setLoggingOut(false)
    }
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <img className="logo" src="/mark-cream.svg" alt="Orveil" />
          <span>
            Orveil Console<small>{tr("enterprise_ai_gateway")}</small>
          </span>
        </div>
        {groups.map((group) => (
          <div className="nav-group" key={group.title}>
            <div className="nav-title">{group.title}</div>
            <nav className="nav">
              {group.items.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  end={'end' in item ? item.end : undefined}
                  className={({ isActive }) => (isActive ? 'active' : '')}
                >
                  <span className="dot" />
                  {item.label}
                </NavLink>
              ))}
            </nav>
          </div>
        ))}
        <div className="sidebar-foot">
          <div>Orveil Console v0.1</div>
        </div>
      </aside>
      <div className="main">
        <header className="topbar">
          <h1>{titleFor(pathname)}</h1>
          <span className="crumb">{tr("console")} / {titleFor(pathname)}</span>
          <div className="topbar-right">
            <LocaleSwitch />
            <div className="topbar-user">
              <span className="avatar" aria-hidden="true">
                {initials(displayName, tr("a"))}
              </span>
              <div className="who">
                <div>{displayName}</div>
                <small>{user?.email}</small>
              </div>
            </div>
            <button className="btn sm" type="button" onClick={() => void handleLogout()} disabled={loggingOut}>
              {loggingOut ? tr("signing_out") : tr("sign_out")}
            </button>
          </div>
        </header>
        <main className="content">
          <Outlet />
        </main>
      </div>
    </div>
  )
}
