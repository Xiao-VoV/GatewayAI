import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'

export type ThemeMode = 'system' | 'light' | 'dark'
export type Locale = 'zh-CN' | 'en-US'

interface PreferencesValue {
  theme: ThemeMode
  resolvedTheme: 'light' | 'dark'
  locale: Locale
  setTheme: (theme: ThemeMode) => void
  setLocale: (locale: Locale) => void
  t: (key: string) => string
}

const STORAGE_KEY = 'gw-client-ui-v1'

const messages: Record<Locale, Record<string, string>> = {
  'zh-CN': {
    'app.workspace': '工作台',
    'nav.home': '首页',
    'nav.tools': 'AI 工具',
    'nav.routing': '路由',
    'nav.models': '模型',
    'nav.devices': '我的设备',
    'nav.settings': '设置',
    'nav.recovery': '诊断与恢复',
    'status.connected': '已连接',
    'status.disconnected': '连接异常',
    'action.sync': '同步',
    'action.syncing': '同步中…',
    'action.configureTools': '配置工具',
    'action.diagnostics': '诊断',
    'action.refresh': '刷新',
    'theme.light': '浅色',
    'theme.dark': '深色',
    'theme.system': '跟随系统',
    'theme.toggle': '切换主题',
    'settings.title': '设置',
    'settings.desc': '连接信息、客户端版本与个人偏好。',
    'settings.appearance': '外观与语言',
    'settings.theme': '主题',
    'settings.themeHint': '主题保存在本机，不随账号同步。',
    'settings.language': '界面语言',
    'settings.languageHint': '部分内容会随语言切换；功能数据保持服务端口径。',
    'settings.connection': '连接',
    'settings.client': '客户端',
    'settings.window': '窗口与托盘',
    'settings.security': '安全说明',
    'settings.windowText': '关闭窗口会隐藏到系统托盘，本地路由与工具配置继续运行；托盘菜单可显示窗口或退出。单实例运行：再次打开应用会聚焦已有窗口；窗口大小/位置自动记忆。',
    'settings.autostart': '开机自动启动',
    'settings.autostartDesktopOnly': '仅在桌面版可用',
    'settings.inputSync': '立即同步',
    'onboarding.welcome.eyebrow': 'ORVEIL AI GATEWAY',
    'onboarding.welcome.title': '让 AI 工具安全接入企业网关',
    'onboarding.welcome.desc': '一次配对，把本机的 Claude Code、Codex、Gemini CLI 等工具统一接入企业模型与权限体系。',
    'onboarding.welcome.start': '开始使用',
    'onboarding.welcome.heading': '这台设备还没有连接',
    'onboarding.welcome.sub': '向管理员获取一次性配对码，然后连接企业网关。',
    'onboarding.welcome.cta': '使用配对码连接',
    'onboarding.welcome.note': '仅支持管理员已启用的企业网关；首次连接需要网络可达。',
    'onboarding.connect.eyebrow': '设备连接',
    'onboarding.connect.title': '连接企业网关',
    'onboarding.connect.desc': '使用管理员提供的一次性配对码注册本机。连接成功后，Key 和设备令牌由本地路由统一管理。',
    'onboarding.connect.heading': '注册这台设备',
    'onboarding.connect.sub': '填写企业网关地址和配对码，设备名会显示在管理台。',
    'onboarding.connect.server': '企业网关地址',
    'onboarding.connect.code': '配对码',
    'onboarding.connect.deviceName': '设备名称',
    'onboarding.connect.submit': '连接并注册',
    'onboarding.connect.busy': '正在连接…',
    'onboarding.back': '返回上一步',
  },
  'en-US': {
    'app.workspace': 'Workspace',
    'nav.home': 'Home',
    'nav.tools': 'AI Tools',
    'nav.routing': 'Routing',
    'nav.models': 'Models',
    'nav.devices': 'My Device',
    'nav.settings': 'Settings',
    'nav.recovery': 'Diagnostics',
    'status.connected': 'Connected',
    'status.disconnected': 'Disconnected',
    'action.sync': 'Sync',
    'action.syncing': 'Syncing…',
    'action.configureTools': 'Configure tools',
    'action.diagnostics': 'Diagnostics',
    'action.refresh': 'Refresh',
    'theme.light': 'Light',
    'theme.dark': 'Dark',
    'theme.system': 'System',
    'theme.toggle': 'Toggle theme',
    'settings.title': 'Settings',
    'settings.desc': 'Connection, client version and personal preferences.',
    'settings.appearance': 'Appearance & language',
    'settings.theme': 'Theme',
    'settings.themeHint': 'Preferences are stored locally, not synced with your account.',
    'settings.language': 'Language',
    'settings.languageHint': 'Some interface copy follows the language; data stays in gateway terms.',
    'settings.connection': 'Connection',
    'settings.client': 'Client',
    'settings.window': 'Window & tray',
    'settings.security': 'Security',
    'settings.windowText': 'Closing the window hides it to the tray; local routing and tool configuration keep running. Use the tray menu to show or quit. Single-instance and window-state are enabled.',
    'settings.autostart': 'Launch at login',
    'settings.autostartDesktopOnly': 'Desktop app only',
    'settings.inputSync': 'Sync now',
    'onboarding.welcome.eyebrow': 'ORVEIL AI GATEWAY',
    'onboarding.welcome.title': 'Connect your AI tools to the enterprise gateway',
    'onboarding.welcome.desc': 'Pair once to bring Claude Code, Codex, Gemini CLI and other local tools under enterprise models and access control.',
    'onboarding.welcome.start': 'GET STARTED',
    'onboarding.welcome.heading': 'This device is not connected',
    'onboarding.welcome.sub': 'Ask your administrator for a one-time pairing code, then connect.',
    'onboarding.welcome.cta': 'Connect with pairing code',
    'onboarding.welcome.note': 'Requires a gateway enabled by your administrator; network access is needed on first connection.',
    'onboarding.connect.eyebrow': 'DEVICE CONNECTION',
    'onboarding.connect.title': 'Connect to your gateway',
    'onboarding.connect.desc': 'Register this device with a one-time pairing code. The local router will manage keys and device tokens for you.',
    'onboarding.connect.heading': 'Register this device',
    'onboarding.connect.sub': 'Enter the gateway address and pairing code. The device name appears in the admin console.',
    'onboarding.connect.server': 'Gateway URL',
    'onboarding.connect.code': 'Pairing code',
    'onboarding.connect.deviceName': 'Device name',
    'onboarding.connect.submit': 'Connect & register',
    'onboarding.connect.busy': 'Connecting…',
    'onboarding.back': 'Back',
  },
}

function readStored(): { theme: ThemeMode; locale: Locale } {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return { theme: 'system', locale: 'zh-CN' }
    const parsed = JSON.parse(raw) as { theme?: ThemeMode; locale?: Locale }
    return {
      theme: parsed.theme === 'light' || parsed.theme === 'dark' ? parsed.theme : 'system',
      locale: parsed.locale === 'en-US' ? 'en-US' : 'zh-CN',
    }
  } catch {
    return { theme: 'system', locale: 'zh-CN' }
  }
}

const UiPreferencesContext = createContext<PreferencesValue | null>(null)

export function UiPreferencesProvider({ children }: { children: ReactNode }) {
  const initial = readStored()
  const [theme, setThemeState] = useState<ThemeMode>(initial.theme)
  const [locale, setLocaleState] = useState<Locale>(initial.locale)
  const [systemDark, setSystemDark] = useState(() => window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? false)

  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const listener = (event: MediaQueryListEvent) => setSystemDark(event.matches)
    media.addEventListener('change', listener)
    return () => media.removeEventListener('change', listener)
  }, [])

  const resolvedTheme: 'light' | 'dark' = theme === 'system' ? (systemDark ? 'dark' : 'light') : theme

  useEffect(() => {
    document.documentElement.dataset.theme = resolvedTheme
    document.documentElement.lang = locale
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ theme, locale }))
    } catch {
      // preferences are non-critical
    }
  }, [theme, locale, resolvedTheme])

  const setTheme = useCallback((next: ThemeMode) => setThemeState(next), [])
  const setLocale = useCallback((next: Locale) => setLocaleState(next), [])
  const t = useCallback(
    (key: string) => messages[locale][key] ?? messages['zh-CN'][key] ?? key,
    [locale],
  )

  const value = useMemo<PreferencesValue>(
    () => ({ theme, resolvedTheme, locale, setTheme, setLocale, t }),
    [theme, resolvedTheme, locale, setTheme, setLocale, t],
  )
  return <UiPreferencesContext.Provider value={value}>{children}</UiPreferencesContext.Provider>
}

export function useUiPreferences(): PreferencesValue {
  const value = useContext(UiPreferencesContext)
  if (!value) throw new Error('useUiPreferences must be used inside UiPreferencesProvider')
  return value
}
