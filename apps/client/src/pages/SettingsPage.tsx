import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { CLIENT_VERSION, detectPlatform, useClient } from '../auth/ClientProvider'
import { isDesktopRuntime } from '../platform/secureStore'
import { disable as disableAutostart, enable as enableAutostart, isEnabled as isAutostartEnabled } from '@tauri-apps/plugin-autostart'
import { PageHead } from '../components/ui'
import { useUiPreferences } from '../ui/preferences'
import type { Locale, ThemeMode } from '../ui/preferences'

export function SettingsPage() {
  const { snapshot, session, syncing, reload, forget, localRouter, routerError } = useClient()
  const { locale, setLocale, theme, setTheme, t } = useUiPreferences()
  const en = locale === 'en-US'
  const navigate = useNavigate()
  const [confirmForget, setConfirmForget] = useState(false)
  const [autostart, setAutostart] = useState<boolean | null>(null)
  const [autostartBusy, setAutostartBusy] = useState(false)
  const [autostartError, setAutostartError] = useState<string | null>(null)

  const copy = locale === 'en-US'
    ? {
        diagnostics: 'Diagnostics',
        sync: 'Sync now',
        syncing: 'Syncing…',
        appearance: 'Appearance & language',
        theme: 'Theme',
        language: 'Language',
        themeHint: 'Stored on this device and applied instantly.',
        languageHint: 'Switches the interface language where translations are available.',
        connection: 'Connection',
        gateway: 'Gateway',
        device: 'Device',
        user: 'User',
        localRouter: 'Local router',
        notRunning: 'Not running in browser preview',
        lastSync: 'Last sync',
        catalog: 'Catalog version',
        client: 'Client',
        version: 'Version',
        platform: 'Platform',
        token: 'Token',
        tokenNote: 'Short-lived; refreshed automatically. Never shown in the UI.',
        credentialStore: 'Credentials',
        keychain: 'System keychain (macOS Keychain)',
        sessionStore: 'Browser sessionStorage (dev preview)',
        network: 'Network',
        networkValue: 'Direct to gateway (local router / Tauri native integration)',
        window: 'Window & tray',
        windowText: 'Closing the window hides it to the tray; local routing and tool configuration keep running. Use the tray menu to show or quit. Single-instance and window-state are enabled.',
        autostart: 'Launch at login',
        desktopOnly: 'Desktop app only',
        loading: 'Loading…',
        on: 'On',
        off: 'Off',
        security: 'Security',
        securityText: 'The server returns the device secret and API key only once. The client never displays the API key or reads conversation content. Desktop stores credentials in the system keychain; browser preview uses sessionStorage and clears on close.',
        clearPairing: 'Remove pairing',
        clearPairingText: 'Removing local credentials requires a new pairing code from an administrator. Unbinding on the admin side also invalidates this device token/key.',
        confirmClear: 'Remove this pairing?',
        clear: 'Remove',
        cancel: 'Cancel',
      }
    : {
        diagnostics: '诊断与恢复',
        sync: '立即同步',
        syncing: '同步中…',
        appearance: '外观与语言',
        theme: '主题',
        language: '语言',
        themeHint: '保存在本机并即时生效。',
        languageHint: '切换已提供翻译的界面文案。',
        connection: '连接',
        gateway: '网关地址',
        device: '设备',
        user: '用户',
        localRouter: '本地路由',
        notRunning: '未运行',
        lastSync: '最近同步',
        catalog: '目录版本',
        client: '客户端',
        version: '版本',
        platform: '平台',
        token: 'Token',
        tokenNote: '短期有效，访问前自动刷新；界面不展示明文。',
        credentialStore: '凭据存储',
        keychain: '系统钥匙串（macOS Keychain）',
        sessionStore: '浏览器 sessionStorage（开发预览）',
        network: '网络代理',
        networkValue: '直连企业网关（本地路由/Tauri 原生集成）',
        window: '窗口与托盘',
        windowText: '关闭窗口会隐藏到系统托盘，本地路由与工具配置继续运行；托盘菜单可显示窗口或退出。单实例运行：再次打开应用会聚焦已有窗口；窗口大小/位置自动记忆。',
        autostart: '开机自动启动',
        desktopOnly: '仅在桌面版可用',
        loading: '读取中…',
        on: '已开启',
        off: '已关闭',
        security: '安全说明',
        securityText: '配对成功后服务端只返回一次设备密钥与 API Key；客户端不展示 API Key 明文，也不读取对话内容。桌面版把两者写入系统钥匙串；浏览器预览放 sessionStorage（仅在标签页生命周期），关闭即失。',
        clearPairing: '清除配对',
        clearPairingText: '清除本机凭据后，需要管理员重新生成配对码才能再次接入；管理员侧解绑设备也会使本机 token/Key 失效。',
        confirmClear: '确定清除本机配对？',
        clear: '确认清除',
        cancel: '取消',
      }

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

  const themeOptions: { id: ThemeMode; label: string }[] = [
    { id: 'system', label: t('theme.system') },
    { id: 'light', label: t('theme.light') },
    { id: 'dark', label: t('theme.dark') },
  ]
  const localeOptions: { id: Locale; label: string }[] = [
    { id: 'zh-CN', label: '中文' },
    { id: 'en-US', label: 'English' },
  ]

  return (
    <>
      <PageHead
        title={t('settings.title')}
        desc={t('settings.desc')}
        actions={
          <>
            <button className="btn" type="button" onClick={() => navigate('/recovery')}>
              {copy.diagnostics}
            </button>
            <button className="btn" type="button" onClick={() => void reload()} disabled={syncing}>
              {syncing ? copy.syncing : copy.sync}
            </button>
          </>
        }
      />

      <div className="card appearance-card">
        <div className="row between">
          <h3>{copy.appearance}</h3>
          <span className="badge accent">{locale === 'zh-CN' ? '本机设置' : 'Local only'}</span>
        </div>
        <div className="preference-grid">
          <div className="preference-item">
            <span className="preference-label">{copy.theme}</span>
            <div className="segmented">
              {themeOptions.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  className={theme === option.id ? 'on' : ''}
                  onClick={() => setTheme(option.id)}
                >
                  {option.label}
                </button>
              ))}
            </div>
            <span className="hint">{copy.themeHint}</span>
          </div>
          <div className="preference-item">
            <span className="preference-label">{copy.language}</span>
            <div className="segmented">
              {localeOptions.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  className={locale === option.id ? 'on' : ''}
                  onClick={() => setLocale(option.id)}
                >
                  {option.label}
                </button>
              ))}
            </div>
            <span className="hint">{copy.languageHint}</span>
          </div>
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <h3>{copy.connection}</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{copy.gateway}</dt>
            <dd><code className="kbd">{session?.serverUrl}</code></dd>
            <dt>{copy.device}</dt>
            <dd>{snapshot.device.name}</dd>
            <dt>{copy.user}</dt>
            <dd>{snapshot.user.display_name || snapshot.user.email} · {snapshot.user.email}</dd>
            <dt>{copy.localRouter}</dt>
            <dd>
              {localRouter ? <code className="kbd">{localRouter.baseUrl}</code> : <span className="hint">{copy.notRunning}{!en && routerError ? ` · ${routerError}` : ''}</span>}
            </dd>
            <dt>{copy.lastSync}</dt>
            <dd>{new Date(snapshot.syncedAt).toLocaleString(locale)}</dd>
            <dt>{copy.catalog}</dt>
            <dd>{snapshot.catalogVersion || '—'}</dd>
          </dl>
        </div>
        <div className="card">
          <h3>{copy.client}</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{copy.version}</dt>
            <dd>v{CLIENT_VERSION}</dd>
            <dt>{copy.platform}</dt>
            <dd>{detectPlatform()}</dd>
            <dt>{copy.token}</dt>
            <dd>{copy.tokenNote}</dd>
            <dt>{copy.credentialStore}</dt>
            <dd>{isDesktopRuntime() ? copy.keychain : copy.sessionStore}</dd>
            <dt>{copy.network}</dt>
            <dd>{copy.networkValue}</dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <h3>{copy.window}</h3>
        <p className="sub" style={{ marginTop: 4 }}>{copy.windowText}</p>
        <div className="row" style={{ marginTop: 12 }}>
          <label className="chk">
            <input
              type="checkbox"
              checked={autostart === true}
              disabled={!isDesktopRuntime() || autostartBusy || autostart === null}
              onChange={() => void toggleAutostart()}
            />
            {copy.autostart}
          </label>
          <span className="hint">
            {!isDesktopRuntime() ? copy.desktopOnly : autostart === null ? copy.loading : autostart ? copy.on : copy.off}
          </span>
        </div>
        {autostartError && <div className="form-error" style={{ marginTop: 8 }}>{autostartError}</div>}
      </div>

      <div className="card">
        <h3>{copy.security}</h3>
        <p className="sub" style={{ marginTop: 4 }}>{copy.securityText}</p>
      </div>

      <div className="card">
        <h3>{copy.clearPairing}</h3>
        <p className="sub" style={{ marginTop: 4 }}>{copy.clearPairingText}</p>
        {confirmForget ? (
          <div className="row" style={{ marginTop: 10 }}>
            <span className="hint">{copy.confirmClear}</span>
            <button className="btn danger" type="button" onClick={handleForget}>{copy.clear}</button>
            <button className="btn" type="button" onClick={() => setConfirmForget(false)}>{copy.cancel}</button>
          </div>
        ) : (
          <button className="btn danger" type="button" onClick={() => setConfirmForget(true)} style={{ marginTop: 10 }}>
            {copy.clearPairing}
          </button>
        )}
      </div>
    </>
  )
}
