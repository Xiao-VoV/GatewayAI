import { useState } from 'react'
import { NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom'
import { CLIENT_VERSION, useClient } from '../auth/ClientProvider'

const nav = [
  { to: '/', label: '首页', end: true },
  { to: '/tools', label: 'AI 工具' },
  { to: '/routing', label: '路由' },
  { to: '/models', label: '模型' },
  { to: '/devices', label: '我的设备' },
  { to: '/settings', label: '设置' },
]

const titles: Record<string, string> = {
  '/': '首页',
  '/tools': 'AI 工具',
  '/routing': '路由',
  '/models': '模型',
  '/devices': '我的设备',
  '/settings': '设置',
}

export function ClientShell() {
  const client = useClient()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const [refreshing, setRefreshing] = useState(false)

  async function handleRefresh() {
    setRefreshing(true)
    try {
      await client.reload()
    } finally {
      setRefreshing(false)
    }
  }

  const title = titles[pathname] ?? 'gw client'
  return (
    <div className="device-page">
      <div className="window">
        <div className="window-bar">
          <div className="dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </div>
          <div className="window-title">gw client · {title}</div>
          <div className="right">
            <span>
              {client.phase === 'connected' ? `已连接 · ${client.snapshot?.device.platform ?? ''}` : '未连接'}
            </span>
            <button className="btn sm ghost" type="button" onClick={() => void handleRefresh()} disabled={refreshing}>
              {refreshing ? '同步中…' : '同步'}
            </button>
          </div>
        </div>
        <div className="window-body">
          <nav className="window-nav">
            <div className="nav-title">工作区</div>
            {nav.map((item) => (
              <NavLink key={item.to} to={item.to} end={item.end} className={({ isActive }) => (isActive ? 'active' : '')}>
                <span
                  className="dot"
                  style={{ width: 6, height: 6, borderRadius: 999, background: 'currentColor', opacity: 0.5 }}
                />
                {item.label}
              </NavLink>
            ))}
            <div className="foot">
              设备 {client.snapshot?.device.id.slice(0, 8)}
              <button className="dev-link" type="button" onClick={() => navigate('/settings')}>
                连接信息
              </button>
            </div>
          </nav>
          <main className="window-main">
            <Outlet />
          </main>
        </div>
        <div className="statusbar">
          <span>企业网关 {client.session?.serverUrl}</span>
          <span className="sep">·</span>
          <span>{client.snapshot?.device.name}</span>
          <span className="sep">·</span>
          <span>Key {client.snapshot?.key.key_prefix ?? '—'}</span>
          <span className="right">client v{CLIENT_VERSION}</span>
        </div>
      </div>
    </div>
  )
}
