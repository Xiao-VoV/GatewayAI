import { useState } from 'react'
import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { useAuth } from '../auth/AuthProvider'

const groups = [
  {
    title: '运营',
    items: [
      { to: '/', label: '总览', end: true },
      { to: '/logs', label: '请求日志' },
      { to: '/audit', label: '审计' },
    ],
  },
  {
    title: '资源',
    items: [
      { to: '/providers', label: '供应商' },
      { to: '/models', label: '模型目录' },
      { to: '/tool-defaults', label: '工具默认' },
      { to: '/catalog-preview', label: '员工目录预览' },
      { to: '/keys', label: '虚拟 Key' },
      { to: '/devices', label: '设备与配对' },
      { to: '/groups', label: '用户与用户组' },
    ],
  },
  {
    title: '系统',
    items: [
      { to: '/settings', label: '设置' },
      { to: '/account', label: '账号与会话' },
    ],
  },
]

const titles: Record<string, string> = {
  '/': '总览',
  '/providers': '供应商',
  '/models': '模型目录',
  '/tool-defaults': '工具默认',
  '/catalog-preview': '员工目录预览',
  '/keys': '虚拟 Key',
  '/devices': '设备与配对',
  '/groups': '用户与用户组',
  '/logs': '请求日志',
  '/audit': '审计',
  '/settings': '设置',
  '/account': '账号与会话',
}

function titleFor(path: string) {
  if (path.startsWith('/providers/')) return '供应商详情'
  if (path.startsWith('/models/')) return '模型组详情'
  return titles[path] ?? '管理台'
}

function initials(name: string) {
  const value = name.trim()
  if (!value) return '管'
  return value.slice(0, 1).toUpperCase()
}

export function AdminShell() {
  const { session, logout } = useAuth()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [loggingOut, setLoggingOut] = useState(false)

  const user = session?.user
  const displayName = user?.display_name || user?.email || '管理员'

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
          <span className="logo">G</span>
          <span>
            gw admin<small>AI Gateway 管理台</small>
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
          <div>gw admin v0.1 · MVP</div>
        </div>
      </aside>
      <div className="main">
        <header className="topbar">
          <h1>{titleFor(pathname)}</h1>
          <span className="crumb">管理台 / {titleFor(pathname)}</span>
          <div className="topbar-right">
            <div className="topbar-user">
              <span className="avatar" aria-hidden="true">
                {initials(displayName)}
              </span>
              <div className="who">
                <div>{displayName}</div>
                <small>{user?.email}</small>
              </div>
            </div>
            <button className="btn sm ghost" type="button" onClick={() => void handleLogout()} disabled={loggingOut}>
              {loggingOut ? '退出中…' : '退出'}
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
