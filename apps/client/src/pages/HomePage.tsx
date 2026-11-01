import { Link } from 'react-router-dom'
import { useClient } from '../auth/ClientProvider'
import { Badge, PageHead, Stat } from '../components/ui'

function greeting() {
  const hour = new Date().getHours()
  if (hour < 6) return '夜深了'
  if (hour < 12) return '早上好'
  if (hour < 18) return '下午好'
  return '晚上好'
}

export function HomePage() {
  const { snapshot, localRouter, routerError } = useClient()
  if (!snapshot) return null
  const usage = snapshot.usage
  const maxTrend = usage ? Math.max(1, ...usage.trend.map((day) => day.requests)) : 1
  const primaryTool = snapshot.tools[0]
  const recentModels = snapshot.models.slice(0, 4)

  return (
    <>
      <PageHead
        title={`${greeting()}，${snapshot.user.display_name || snapshot.user.email.split('@')[0]}`}
        desc="设备已接入企业网关；模型目录来自本组发布。"
      />

      <div className="client-status">
        <span className="dot" aria-hidden="true" />
        <span>网关连接正常</span>
        <span className="badge ok">设备 {snapshot.device.status}</span>
        {localRouter ? (
          <span className="badge accent">本地路由 :{localRouter.port}</span>
        ) : (
          <span className="badge warn" title={routerError ?? ''}>
            本地路由未启动
          </span>
        )}
        <span className="right">同步于 {new Date(snapshot.syncedAt).toLocaleTimeString('zh-CN')}</span>
      </div>

      <div className="grid cols-4">
        <Stat label="设备" hint={snapshot.device.platform}>
          {snapshot.device.name}
        </Stat>
        <Stat label="已发布模型" hint={snapshot.catalogVersion || 'catalog'}>
          {snapshot.models.length}
        </Stat>
        <Stat label="虚拟 Key" hint={snapshot.key.expires_at ? '有过期时间' : '永不过期'}>
          {snapshot.key.key_prefix}
        </Stat>
        <Stat label="工具默认" hint={primaryTool ? primaryTool.tool_key : '未配置'}>
          {primaryTool ? primaryTool.default_model : '—'}
        </Stat>
      </div>

      {usage ? (
        <>
          <div className="grid cols-4">
            <Stat label="今日请求" hint={`成功 ${usage.today.success_requests} · 拒绝 ${usage.today.rejected_requests}`}>
              {usage.today.requests}
            </Stat>
            <Stat label="今日花费" hint={usage.currency}>
              ¥{Number(usage.today.cost).toFixed(2)}
            </Stat>
            <Stat
              label="今日错误率"
              hint={`错误 ${usage.today.error_requests} · 断开 ${usage.today.client_disconnect_requests}`}
            >
              {((usage.today.error_rate ?? 0) * 100).toFixed(2)}%
            </Stat>
            <Stat label={`最近 ${usage.days} 天请求`} hint={`花费 ¥${Number(usage.totals.cost).toFixed(2)}`}>
              {usage.totals.requests}
            </Stat>
          </div>
          <div className="card">
            <div className="row between">
              <h3>用量趋势</h3>
              <span className="sub">
                {usage.trend.length} 天 · 更新于 {new Date(usage.generated_at).toLocaleTimeString('zh-CN')}
              </span>
            </div>
            <div className="bars" role="img" aria-label="最近用量趋势">
              {usage.trend.map((day) => (
                <i
                  key={day.day}
                  className={day.day === usage.trend[usage.trend.length - 1]?.day ? 'hi' : ''}
                  style={{ height: `${day.requests === 0 ? 6 : Math.max(8, Math.round((day.requests / maxTrend) * 100))}%` }}
                  title={`${day.day} · ${day.requests} 请求 · ¥${day.cost}`}
                />
              ))}
            </div>
            <div className="bar-labels">
              {usage.trend.map((day) => (
                <span key={day.day}>{day.day.slice(5)}</span>
              ))}
            </div>
            <div className="hint" style={{ marginTop: 8 }}>
              Token：输入 {usage.totals.input_tokens} · 输出 {usage.totals.output_tokens}；仅展示本人用量。
            </div>
          </div>
        </>
      ) : (
        <div className="note">用量暂不可用；设备与目录功能不受影响。</div>
      )}

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>可用模型</h3>
            <Link className="btn sm" to="/models">
              查看全部
            </Link>
          </div>
          {recentModels.length === 0 ? (
            <p className="hint" style={{ marginTop: 10 }}>
              本组还没有已发布模型；请联系管理员发布模型。
            </p>
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
            <h3>工具默认模型</h3>
            <Badge tone="accent">{snapshot.tools.length} 个工具</Badge>
          </div>
          {snapshot.tools.length === 0 ? (
            <p className="hint" style={{ marginTop: 10 }}>
              管理员尚未为该用户/组配置工具默认；可在工具内选择模型。
            </p>
          ) : (
            <div style={{ marginTop: 8 }}>
              {snapshot.tools.map((tool) => (
                <div className="status-row" key={tool.tool_key}>
                  <span>
                    <code className="kbd">{tool.tool_key}</code>
                    <span className="hint"> → {tool.default_model}</span>
                  </span>
                  <span style={{ marginLeft: 'auto' }} className="hint">
                    {tool.allow_switch ? `可切换 ${tool.switchable_models.length}` : '固定'}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <h3>本机接入</h3>
          <p className="sub" style={{ marginTop: 4 }}>
            {localRouter
              ? `本地路由 ${localRouter.baseUrl} 已运行；Claude Code 可在「AI 工具」页写入配置。`
              : '本地路由未运行；AI 工具配置暂时无法生效。可在「AI 工具」页查看原因。'}
          </p>
        </div>
        <div className="card">
          <div className="row between">
            <h3>快捷操作</h3>
          </div>
          <div className="row" style={{ marginTop: 10, flexWrap: 'wrap' }}>
            <Link className="btn" to="/tools">
              扫描 AI 工具
            </Link>
            <Link className="btn" to="/models">
              浏览模型
            </Link>
            <Link className="btn" to="/devices">
              设备与 Key
            </Link>
            <Link className="btn" to="/settings">
              连接设置
            </Link>
          </div>
        </div>
      </div>
    </>
  )
}
