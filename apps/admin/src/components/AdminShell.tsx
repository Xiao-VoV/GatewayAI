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
      title: tr('运营', 'Operations'),
      items: [
        { to: '/', label: tr('总览', 'Overview'), end: true },
        { to: '/logs', label: tr('请求日志', 'Request logs') },
        { to: '/audit', label: tr('审计', 'Audit') },
      ],
    },
    {
      title: tr('资源', 'Resources'),
      items: [
        { to: '/providers', label: tr('供应商', 'Providers') },
        { to: '/models', label: tr('模型目录', 'Model catalog') },
        { to: '/tool-defaults', label: tr('工具默认', 'Tool defaults') },
        { to: '/catalog-preview', label: tr('员工目录预览', 'Employee catalog') },
        { to: '/keys', label: tr('虚拟 Key', 'Virtual keys') },
        { to: '/devices', label: tr('设备与配对', 'Devices & pairing') },
        { to: '/groups', label: tr('用户与用户组', 'Users & groups') },
      ],
    },
    {
      title: tr('系统', 'System'),
      items: [
        { to: '/settings', label: tr('设置', 'Settings') },
        { to: '/account', label: tr('账号与会话', 'Account & session') },
      ],
    },
  ]

  const titles: Record<string, string> = {
    '/': tr('总览', 'Overview'),
    '/providers': tr('供应商', 'Providers'),
    '/models': tr('模型目录', 'Model catalog'),
    '/tool-defaults': tr('工具默认', 'Tool defaults'),
    '/catalog-preview': tr('员工目录预览', 'Employee catalog'),
    '/keys': tr('虚拟 Key', 'Virtual keys'),
    '/devices': tr('设备与配对', 'Devices & pairing'),
    '/groups': tr('用户与用户组', 'Users & groups'),
    '/logs': tr('请求日志', 'Request logs'),
    '/audit': tr('审计', 'Audit'),
    '/settings': tr('设置', 'Settings'),
    '/account': tr('账号与会话', 'Account & session'),
  }

  function titleFor(path: string) {
    if (path.startsWith('/providers/')) return tr('供应商详情', 'Provider detail')
    if (path.startsWith('/models/')) return tr('模型组详情', 'Model group detail')
    return titles[path] ?? tr('管理台', 'Console')
  }

  const user = session?.user
  const displayName = user?.display_name || user?.email || tr('管理员', 'Administrator')

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
            Orveil Console<small>{tr('企业 AI 网关控制台', 'Enterprise AI Gateway')}</small>
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
          <span className="crumb">{tr('管理台', 'Console')} / {titleFor(pathname)}</span>
          <div className="topbar-right">
            <LocaleSwitch />
            <div className="topbar-user">
              <span className="avatar" aria-hidden="true">
                {initials(displayName, tr('管', 'A'))}
              </span>
              <div className="who">
                <div>{displayName}</div>
                <small>{user?.email}</small>
              </div>
            </div>
            <button className="btn sm" type="button" onClick={() => void handleLogout()} disabled={loggingOut}>
              {loggingOut ? tr('退出中…', 'Signing out…') : tr('退出', 'Sign out')}
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
