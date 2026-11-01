import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useClient } from '../auth/ClientProvider'
import { nativeHttpRequest } from '../platform/nativeHttp'
import { isDesktopRuntime } from '../platform/secureStore'
import { Badge, PageHead } from '../components/ui'
import { useUiPreferences } from '../ui/preferences'

const adapters = ['OpenAI Chat', 'Anthropic Messages', 'OpenAI Responses', 'Gemini generateContent']

export function RoutingPage() {
  const { snapshot, session, localRouter, routerError, syncing, reload } = useClient()
  const { locale } = useUiPreferences()
  const en = locale === 'en-US'
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<string | null>(null)
  const [testError, setTestError] = useState<string | null>(null)

  if (!snapshot) return null

  async function testLocalRoute() {
    if (!localRouter) return
    setTesting(true)
    setTestResult(null)
    setTestError(null)
    const started = performance.now()
    try {
      if (!isDesktopRuntime()) {
        setTestError(en ? 'Route test requires the Tauri desktop app' : '链路测试需要 Tauri 桌面版')
        return
      }
      const response = await nativeHttpRequest(
        'GET',
        `${localRouter.baseUrl}/v1/models`,
        { Authorization: 'Bearer gw-local-device' },
        null,
      )
      const latency = Math.round(performance.now() - started)
      if (response.status < 200 || response.status >= 300) {
        setTestError(en ? `Local router returned HTTP ${response.status}` : `本地路由返回 HTTP ${response.status}`)
        return
      }
      const payload = JSON.parse(response.body) as { data?: unknown[] }
      setTestResult(en ? `Local router → gateway OK · ${payload.data?.length ?? 0} models · ${latency}ms` : `本地路由 → 企业网关正常 · ${payload.data?.length ?? 0} 个模型 · ${latency}ms`)
    } catch (error) {
      setTestError(error instanceof Error ? error.message : en ? 'Route test failed' : '链路测试失败')
    } finally {
      setTesting(false)
    }
  }

  return (
    <>
      <PageHead
        title={en ? 'Routing' : '路由'}
        desc={en ? 'The local router securely forwards tool traffic to the gateway. Model allocation and fallback stay with administrators.' : '本地路由只把工具流量安全转发到企业网关；模型分配/回退由管理员在服务端配置。'}
        actions={
          <>
            <button className="btn" type="button" onClick={() => void reload()} disabled={syncing}>
              {syncing ? (en ? 'Refreshing…' : '刷新中…') : en ? 'Refresh' : '刷新'}
            </button>
            <Link className="btn" to="/recovery">
              {en ? 'Diagnostics' : '诊断'}
            </Link>
          </>
        }
      />

      <div className="status-strip">
        <div className="metric">
          <label>{en ? 'Local router' : '本地路由'}</label>
          <strong>
            <span className="dot" style={{ background: localRouter ? 'var(--ok)' : 'var(--bad)' }} />
            {localRouter ? (en ? 'Running' : '运行中') : en ? 'Stopped' : '未运行'}
          </strong>
        </div>
        <div className="metric">
          <label>{en ? 'Address' : '地址'}</label>
          <strong className="mono">{localRouter?.baseUrl ?? '—'}</strong>
        </div>
        <div className="metric">
          <label>{en ? 'Gateway' : '企业网关'}</label>
          <strong className="small">{session?.serverUrl ?? '—'}</strong>
        </div>
        <div className="metric">
          <label>{en ? 'Protocol adaptation' : '协议适配'}</label>
          <strong className="small">{en ? 'Server-side' : '服务端完成'}</strong>
        </div>
      </div>

      {!localRouter && (
        <div className="note warn">
          {en ? 'Local router is not running in browser preview. Tool configuration cannot take effect until the desktop app starts it.' : `本地路由未运行：${routerError ?? '桌面版配对成功后自动启动'}。工具配置暂时无法生效。`}
        </div>
      )}

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{en ? 'Tool default routing' : '工具默认映射'}</h3>
            <Badge tone="accent">{snapshot.tools.length} {en ? 'tools' : '个工具'}</Badge>
          </div>
          {snapshot.tools.length === 0 ? (
            <p className="hint" style={{ marginTop: 10 }}>
              {en ? 'No tool defaults configured yet; choose a model inside each tool.' : '管理员尚未配置工具默认；可在工具内自行选择模型。'}
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
                    {tool.allow_switch ? (en ? `switchable ${tool.switchable_models.length}` : `可切换 ${tool.switchable_models.length}`) : en ? 'fixed' : '固定'}
                  </span>
                </div>
              ))}
            </div>
          )}
          <div className="row" style={{ marginTop: 10 }}>
            <Link className="btn sm" to="/tools">
              {en ? 'Configure tools' : '配置工具'}
            </Link>
          </div>
        </div>

        <div className="card">
          <h3>{en ? 'Protocol support' : '协议能力'}</h3>
          <p className="hint" style={{ marginTop: 4 }}>
            {en ? 'Protocol adaptation runs in the server proxy; the local router only forwards traffic without rewriting bodies.' : '工具协议由服务端 proxy 适配；本地路由只透传，不改写 body。'}
          </p>
          <div className="row" style={{ marginTop: 10, flexWrap: 'wrap' }}>
            {adapters.map((adapter) => (
              <Badge key={adapter}>{adapter}</Badge>
            ))}
          </div>
          <div className="row" style={{ marginTop: 14 }}>
            <button className="btn" type="button" onClick={() => void testLocalRoute()} disabled={!localRouter || testing}>
              {testing ? (en ? 'Testing…' : '测试中…') : en ? 'Test local route' : '测试本地链路'}
            </button>
            {testResult && <span className="hint">{testResult}</span>}
            {testError && <span className="field-error">{testError}</span>}
          </div>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <h3>{en ? 'Group model catalog' : '本组模型目录'}</h3>
          <Link className="btn sm" to="/models">
            {en ? 'View details' : '查看详情'}
          </Link>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{en ? 'Model ID' : '对外模型名'}</th>
                <th>{en ? 'Display name' : '显示名'}</th>
                <th>{en ? 'Capabilities' : '能力'}</th>
                <th>{en ? 'Protocols' : '协议'}</th>
                <th>{en ? 'Context' : '上下文'}</th>
              </tr>
            </thead>
            <tbody>
              {snapshot.models.map((model) => (
                <tr key={model.id}>
                  <td>
                    <code className="kbd">{model.id}</code>
                  </td>
                  <td>{model.display_name}</td>
                  <td>{model.capabilities.join(', ') || '—'}</td>
                  <td>{model.protocols.join(', ') || '—'}</td>
                  <td className="num">{model.context_window ?? '—'}</td>
                </tr>
              ))}
              {snapshot.models.length === 0 && (
                <tr>
                  <td colSpan={5} className="hint">
                    {en ? 'No models published to your group yet.' : '本组还没有已发布模型。'}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="hint" style={{ marginTop: 8 }}>
          {en ? 'The client does not modify model allocation or fallback; ask an administrator. Local router paths: /v1/* and /v1beta/*.' : '客户端不修改模型分配/回退；如需调整请由管理员在管理台操作。本地路由请求路径：`/v1/*`、`/v1beta/*`。'}
        </p>
      </div>
    </>
  )
}
