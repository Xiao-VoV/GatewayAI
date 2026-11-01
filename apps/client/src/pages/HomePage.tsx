import { Link } from 'react-router-dom'
import { useClient } from '../auth/ClientProvider'
import { Badge, Stat } from '../components/ui'
import { useUiPreferences } from '../ui/preferences'

function greeting(locale: string) {
  const hour = new Date().getHours()
  if (locale === 'en-US') {
    if (hour < 6) return 'Working late'
    if (hour < 12) return 'Good morning'
    if (hour < 18) return 'Good afternoon'
    return 'Good evening'
  }
  if (hour < 6) return '夜深了'
  if (hour < 12) return '早上好'
  if (hour < 18) return '下午好'
  return '晚上好'
}

export function HomePage() {
  const { snapshot, localRouter, routerError } = useClient()
  const { locale } = useUiPreferences()
  if (!snapshot) return null
  const en = locale === 'en-US'
  const usage = snapshot.usage
  const maxTrend = usage ? Math.max(1, ...usage.trend.map((day) => day.requests)) : 1
  const hasUsage = usage ? usage.totals.requests > 0 : false
  const primaryTool = snapshot.tools[0]
  const recentModels = snapshot.models.slice(0, 4)

  const copy = en
    ? {
        desc: 'Device connected to the enterprise gateway. Models come from your group publications.',
        gatewayOk: 'Gateway connected',
        device: 'Device',
        publishedModels: 'Published models',
        virtualKey: 'Virtual key',
        toolDefaults: 'Tool defaults',
        expires: 'Has expiry',
        neverExpires: 'No expiry',
        notConfigured: 'Not configured',
        todayRequests: 'Requests today',
        success: 'success',
        rejected: 'rejected',
        todayCost: 'Spend today',
        errorRate: 'Error rate today',
        errors: 'errors',
        disconnects: 'disconnects',
        recentRequests: (days: number) => `Requests · ${days} days`,
        spent: 'spent',
        usageTrend: 'Usage trend',
        days: 'days',
        updated: 'updated',
        noRequests: (days: number) => `No requests in the last ${days} days`,
        tokens: 'Tokens',
        input: 'input',
        output: 'output',
        ownUsage: 'Only your own usage is shown.',
        usageUnavailable: 'Usage is temporarily unavailable. Device and catalog features are unaffected.',
        availableModels: 'Available models',
        viewAll: 'View all',
        noModels: 'No models published to your group yet. Ask an administrator to publish models.',
        tools: 'Tool defaults',
        toolsCount: (count: number) => `${count} tools`,
        noTools: 'No tool defaults configured for you yet; choose models inside each tool.',
        switchable: (count: number) => `switchable ${count}`,
        fixed: 'fixed',
        localAccess: 'Local access',
        routerRunning: (url: string) => `Local router ${url} is running. Configure Claude Code from “AI Tools”.`,
        routerStopped: 'Local router is not running; tool configuration cannot take effect. See “AI Tools” for details.',
        quickActions: 'Quick actions',
        scanTools: 'Scan AI tools',
        browseModels: 'Browse models',
        deviceKey: 'Device & key',
        connectionSettings: 'Connection settings',
      }
    : {
        desc: '设备已接入企业网关；模型目录来自本组发布。',
        gatewayOk: '网关连接正常',
        device: '设备',
        publishedModels: '已发布模型',
        virtualKey: '虚拟 Key',
        toolDefaults: '工具默认',
        expires: '有过期时间',
        neverExpires: '永不过期',
        notConfigured: '未配置',
        todayRequests: '今日请求',
        success: '成功',
        rejected: '拒绝',
        todayCost: '今日花费',
        errorRate: '今日错误率',
        errors: '错误',
        disconnects: '断开',
        recentRequests: (days: number) => `最近 ${days} 天请求`,
        spent: '花费',
        usageTrend: '用量趋势',
        days: '天',
        updated: '更新于',
        noRequests: (days: number) => `最近 ${days} 天暂无请求`,
        tokens: 'Token',
        input: '输入',
        output: '输出',
        ownUsage: '仅展示本人用量。',
        usageUnavailable: '用量暂不可用；设备与目录功能不受影响。',
        availableModels: '可用模型',
        viewAll: '查看全部',
        noModels: '本组还没有已发布模型；请联系管理员发布模型。',
        tools: '工具默认模型',
        toolsCount: (count: number) => `${count} 个工具`,
        noTools: '管理员尚未为该用户/组配置工具默认；可在工具内选择模型。',
        switchable: (count: number) => `可切换 ${count}`,
        fixed: '固定',
        localAccess: '本机接入',
        routerRunning: (url: string) => `本地路由 ${url} 已运行；Claude Code 可在“AI 工具”页写入配置。`,
        routerStopped: '本地路由未运行；AI 工具配置暂时无法生效。可在“AI 工具”页查看原因。',
        quickActions: '快捷操作',
        scanTools: '扫描 AI 工具',
        browseModels: '浏览模型',
        deviceKey: '设备与 Key',
        connectionSettings: '连接设置',
      }

  return (
    <>
      <section className="home-hero">
        <div className="home-hero-copy">
          <span className="eyebrow">GW CLIENT</span>
          <h2>
            {greeting(locale)}，{snapshot.user.display_name || snapshot.user.email.split('@')[0]}
          </h2>
          <p>{copy.desc}</p>
        </div>
        <div className="home-hero-actions">
          <Link className="btn hero primary" to="/tools">
            {copy.scanTools}
          </Link>
          <Link className="btn hero" to="/models">
            {copy.browseModels}
          </Link>
        </div>
      </section>

      <div className="client-status">
        <span className="dot" aria-hidden="true" />
        <span>{copy.gatewayOk}</span>
        <span className="badge ok">
          {copy.device} {snapshot.device.status}
        </span>
        {localRouter ? (
          <span className="badge accent">{en ? 'Router' : '本地路由'} :{localRouter.port}</span>
        ) : (
          <span className="badge warn" title={routerError ?? ''}>
            {en ? 'Router stopped' : '本地路由未启动'}
          </span>
        )}
        <span className="right">
          {copy.updated} {new Date(snapshot.syncedAt).toLocaleTimeString(locale)}
        </span>
      </div>

      <div className="grid cols-4">
        <Stat label={copy.device} hint={snapshot.device.platform}>
          {snapshot.device.name}
        </Stat>
        <Stat label={copy.publishedModels} hint={snapshot.catalogVersion || 'catalog'}>
          {snapshot.models.length}
        </Stat>
        <Stat label={copy.virtualKey} hint={snapshot.key.expires_at ? copy.expires : copy.neverExpires}>
          {snapshot.key.key_prefix}
        </Stat>
        <Stat label={copy.toolDefaults} hint={primaryTool ? primaryTool.tool_key : copy.notConfigured}>
          {primaryTool ? primaryTool.default_model : '—'}
        </Stat>
      </div>

      {usage ? (
        <>
          <div className="grid cols-4">
            <Stat
              label={copy.todayRequests}
              hint={`${copy.success} ${usage.today.success_requests} · ${copy.rejected} ${usage.today.rejected_requests}`}
            >
              {usage.today.requests}
            </Stat>
            <Stat label={copy.todayCost} hint={usage.currency}>
              ¥{Number(usage.today.cost).toFixed(2)}
            </Stat>
            <Stat
              label={copy.errorRate}
              hint={`${copy.errors} ${usage.today.error_requests} · ${copy.disconnects} ${usage.today.client_disconnect_requests}`}
            >
              {((usage.today.error_rate ?? 0) * 100).toFixed(2)}%
            </Stat>
            <Stat label={copy.recentRequests(usage.days)} hint={`${copy.spent} ¥${Number(usage.totals.cost).toFixed(2)}`}>
              {usage.totals.requests}
            </Stat>
          </div>
          <div className="card">
            <div className="row between">
              <h3>{copy.usageTrend}</h3>
              <span className="sub">
                {usage.trend.length} {copy.days} · {copy.updated} {new Date(usage.generated_at).toLocaleTimeString(locale)}
              </span>
            </div>
            {hasUsage ? (
              <>
                <div className="bars" role="img" aria-label={copy.usageTrend}>
                  {usage.trend.map((day) => (
                    <i
                      key={day.day}
                      className={day.day === usage.trend[usage.trend.length - 1]?.day ? 'hi' : ''}
                      style={{ height: `${day.requests === 0 ? 6 : Math.max(8, Math.round((day.requests / maxTrend) * 100))}%` }}
                      title={`${day.day} · ${day.requests} · ¥${day.cost}`}
                    />
                  ))}
                </div>
                <div className="bar-labels">
                  {usage.trend.map((day) => (
                    <span key={day.day}>{day.day.slice(5)}</span>
                  ))}
                </div>
              </>
            ) : (
              <div className="chart-empty">{copy.noRequests(usage.days)}</div>
            )}
            <div className="hint" style={{ marginTop: 8 }}>
              {copy.tokens}: {copy.input} {usage.totals.input_tokens} · {copy.output} {usage.totals.output_tokens}; {copy.ownUsage}
            </div>
          </div>
        </>
      ) : (
        <div className="note">{copy.usageUnavailable}</div>
      )}

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{copy.availableModels}</h3>
            <Link className="btn sm" to="/models">
              {copy.viewAll}
            </Link>
          </div>
          {recentModels.length === 0 ? (
            <p className="hint" style={{ marginTop: 10 }}>{copy.noModels}</p>
          ) : (
            <div style={{ marginTop: 8 }}>
              {recentModels.map((model) => (
                <div className="status-row" key={model.id}>
                  <span>
                    <strong>{model.display_name}</strong>
                    <span className="hint"> · {model.id}</span>
                  </span>
                  <span style={{ marginLeft: 'auto' }}>
                    <Badge>{model.capabilities.slice(0, 2).join(' / ') || 'chat'}</Badge>
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="card">
          <div className="row between">
            <h3>{copy.tools}</h3>
            <Badge tone="accent">{copy.toolsCount(snapshot.tools.length)}</Badge>
          </div>
          {snapshot.tools.length === 0 ? (
            <p className="hint" style={{ marginTop: 10 }}>{copy.noTools}</p>
          ) : (
            <div style={{ marginTop: 8 }}>
              {snapshot.tools.map((tool) => (
                <div className="status-row" key={tool.tool_key}>
                  <span>
                    <code className="kbd">{tool.tool_key}</code>
                    <span className="hint"> → {tool.default_model}</span>
                  </span>
                  <span style={{ marginLeft: 'auto' }} className="hint">
                    {tool.allow_switch ? copy.switchable(tool.switchable_models.length) : copy.fixed}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <h3>{copy.localAccess}</h3>
          <p className="sub" style={{ marginTop: 4 }}>
            {localRouter ? copy.routerRunning(localRouter.baseUrl) : copy.routerStopped}
          </p>
        </div>
        <div className="card">
          <div className="row between">
            <h3>{copy.quickActions}</h3>
          </div>
          <div className="row" style={{ marginTop: 10, flexWrap: 'wrap' }}>
            <Link className="btn" to="/tools">{copy.scanTools}</Link>
            <Link className="btn" to="/models">{copy.browseModels}</Link>
            <Link className="btn" to="/devices">{copy.deviceKey}</Link>
            <Link className="btn" to="/settings">{copy.connectionSettings}</Link>
          </div>
        </div>
      </div>
    </>
  )
}
