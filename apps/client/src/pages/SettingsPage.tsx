import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { CLIENT_VERSION, detectPlatform, useClient } from '../auth/ClientProvider'
import { isDesktopRuntime } from '../platform/secureStore'
import { disable as disableAutostart, enable as enableAutostart, isEnabled as isAutostartEnabled } from '@tauri-apps/plugin-autostart'
import { PageHead } from '../components/ui'

export function SettingsPage() {
  const { snapshot, session, syncing, reload, forget, localRouter, routerError } = useClient()
  const navigate = useNavigate()
  const [confirmForget, setConfirmForget] = useState(false)
  const [autostart, setAutostart] = useState<boolean | null>(null)
  const [autostartBusy, setAutostartBusy] = useState(false)
  const [autostartError, setAutostartError] = useState<string | null>(null)
  useEffect(() => {
    if (!isDesktopRuntime()) return
    isAutostartEnabled()
      .then(setAutostart)
      .catch((error: unknown) => setAutostartError(error instanceof Error ? error.message : '读取开机自启状态失败'))
  }, [])

  async function toggleAutostart() {
    if (!isDesktopRuntime()) return
    setAutostartBusy(true)
    setAutostartError(null)
    try {
      if (autostart) await disableAutostart()
      else await enableAutostart()
      setAutostart(await isAutostartEnabled())
    } catch (error) {
      setAutostartError(error instanceof Error ? error.message : '设置开机自启失败')
    } finally {
      setAutostartBusy(false)
    }
  }

  if (!snapshot) return null

  function handleForget() {
    forget()
    navigate('/welcome', { replace: true })
  }

  return (
    <>
      <PageHead
        title="设置"
        desc="连接信息、客户端版本与安全边界。"
        actions={
          <>
            <button className="btn" type="button" onClick={() => navigate('/recovery')}>
              诊断与恢复
            </button>
            <button className="btn" type="button" onClick={() => void reload()} disabled={syncing}>
              {syncing ? '同步中…' : '立即同步'}
            </button>
          </>
        }
      />

      <div className="grid cols-2">
        <div className="card">
          <h3>连接</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>网关地址</dt>
            <dd>
              <code className="kbd">{session?.serverUrl}</code>
            </dd>
            <dt>设备</dt>
            <dd>{snapshot.device.name}</dd>
            <dt>用户</dt>
            <dd>
              {snapshot.user.display_name || snapshot.user.email} · {snapshot.user.email}
            </dd>
            <dt>本地路由</dt>
            <dd>
              {localRouter ? (
                <code className="kbd">{localRouter.baseUrl}</code>
              ) : (
                <span className="hint">未运行{routerError ? ` · ${routerError}` : ''}</span>
              )}
            </dd>
            <dt>最近同步</dt>
            <dd>{new Date(snapshot.syncedAt).toLocaleString('zh-CN')}</dd>
            <dt>目录版本</dt>
            <dd>{snapshot.catalogVersion || '—'}</dd>
          </dl>
        </div>
        <div className="card">
          <h3>客户端</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>版本</dt>
            <dd>v{CLIENT_VERSION}</dd>
            <dt>平台</dt>
            <dd>{detectPlatform()}</dd>
            <dt>Token</dt>
            <dd>短期有效，访问前自动刷新；界面不展示明文</dd>
            <dt>凭据存储</dt>
            <dd>{isDesktopRuntime() ? '系统钥匙串（macOS Keychain）' : '浏览器 sessionStorage（开发预览）'}</dd>
            <dt>网络代理</dt>
            <dd>直连企业网关（本地路由/Tauri 原生切片后续接入）</dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <h3>窗口与托盘</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          关闭窗口会隐藏到系统托盘，本地路由与工具配置继续运行；托盘菜单可显示窗口或退出。退出应用会同时停止本地路由。
          单实例运行：再次打开应用会聚焦已有窗口；窗口大小/位置自动记忆。
        </p>
        <div className="row" style={{ marginTop: 12 }}>
          <label className="chk">
            <input
              type="checkbox"
              checked={autostart === true}
              disabled={!isDesktopRuntime() || autostartBusy || autostart === null}
              onChange={() => void toggleAutostart()}
            />
            开机自动启动
          </label>
          <span className="hint">
            {!isDesktopRuntime()
              ? '仅在桌面版可用'
              : autostart === null
                ? '读取中…'
                : autostart
                  ? '已开启'
                  : '已关闭'}
          </span>
        </div>
        {autostartError && <div className="form-error" style={{ marginTop: 8 }}>{autostartError}</div>}
      </div>

      <div className="card">
        <h3>安全说明</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          配对成功后服务端只返回一次设备密钥与 API Key；客户端不展示 API Key 明文，也不读取对话内容。
          桌面版把两者写入系统钥匙串；浏览器预览放 sessionStorage（仅在标签页生命周期），关闭即失。
        </p>
      </div>

      <div className="card">
        <h3>清除配对</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          清除本机凭据后，需要管理员重新生成配对码才能再次接入；管理员侧解绑设备也会使本机 token/Key 失效。
        </p>
        {confirmForget ? (
          <div className="row" style={{ marginTop: 10 }}>
            <span className="hint">确定清除本机配对？</span>
            <button className="btn danger" type="button" onClick={handleForget}>
              确认清除
            </button>
            <button className="btn" type="button" onClick={() => setConfirmForget(false)}>
              取消
            </button>
          </div>
        ) : (
          <button className="btn danger" type="button" onClick={() => setConfirmForget(true)} style={{ marginTop: 10 }}>
            清除本机配对
          </button>
        )}
      </div>
    </>
  )
}
