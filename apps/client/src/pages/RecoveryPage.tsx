import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useClient } from '../auth/ClientProvider'
import { isDesktopRuntime } from '../platform/secureStore'
import { localRouterStatus } from '../platform/nativeRouter'
import { nativeHttpRequest } from '../platform/nativeHttp'
import { Badge, PageHead } from '../components/ui'

interface CheckResult {
  id: string
  label: string
  ok: boolean
  detail: string
  latencyMs?: number
}

async function probe(url: string): Promise<{ ok: boolean; status: number; latencyMs: number; detail: string }> {
  const started = performance.now()
  const controller = new AbortController()
  const timer = window.setTimeout(() => controller.abort(), 5000)
  try {
    let status: number
    let text: string
    if (isDesktopRuntime()) {
      const response = await nativeHttpRequest('GET', url, {}, null)
      status = response.status
      text = response.body
    } else {
      const response = await fetch(url, { signal: controller.signal })
      status = response.status
      text = await response.text()
    }
    const ok = status >= 200 && status < 300
    const latencyMs = Math.round(performance.now() - started)
    let detail = `HTTP ${status}`
    if (ok && text) {
      try {
        const payload = JSON.parse(text) as Record<string, unknown>
        detail = typeof payload.status === 'string' ? `HTTP ${status} · ${payload.status}` : detail
      } catch {
        // non-json health output still counts by status
      }
    }
    return { ok, status, latencyMs, detail }
  } catch (error) {
    return {
      ok: false,
      status: 0,
      latencyMs: Math.round(performance.now() - started),
      detail: error instanceof Error ? error.message : '不可达',
    }
  } finally {
    window.clearTimeout(timer)
  }
}

export function RecoveryPage() {
  const client = useClient()
  const navigate = useNavigate()
  const [checks, setChecks] = useState<CheckResult[]>([])
  const [running, setRunning] = useState(false)
  const [copied, setCopied] = useState(false)
  const [confirmForget, setConfirmForget] = useState(false)

  const run = useCallback(async () => {
    if (!client.session) return
    setRunning(true)
    setCopied(false)
    const results: CheckResult[] = []
    const health = await probe(`${client.session.serverUrl}/healthz`)
    results.push({ id: 'health', label: '网关 healthz', ok: health.ok, detail: health.detail, latencyMs: health.latencyMs })
    const ready = await probe(`${client.session.serverUrl}/readyz`)
    results.push({ id: 'ready', label: '网关 readyz（DB）', ok: ready.ok, detail: ready.detail, latencyMs: ready.latencyMs })
    if (isDesktopRuntime()) {
      try {
        const router = await localRouterStatus()
        results.push({
          id: 'router',
          label: '本地路由',
          ok: router.running,
          detail: router.running ? `${router.baseUrl}（127.0.0.1:${router.port}）` : (client.routerError ?? '未运行'),
        })
      } catch (error) {
        results.push({
          id: 'router',
          label: '本地路由',
          ok: false,
          detail: error instanceof Error ? error.message : '状态读取失败',
        })
      }
    } else {
      results.push({ id: 'router', label: '本地路由', ok: false, detail: '浏览器预览不支持（需 Tauri 桌面版）' })
    }
    results.push({
      id: 'catalog',
      label: '模型目录同步',
      ok: client.snapshot !== null,
      detail: client.snapshot
        ? `${client.snapshot.models.length} 个模型 · ${new Date(client.snapshot.syncedAt).toLocaleTimeString('zh-CN')}`
        : '尚未同步',
    })
    setChecks(results)
    setRunning(false)
  }, [client])

  useEffect(() => {
    void run()
  }, [run])

  async function handleRestartRouter() {
    setRunning(true)
    try {
      await client.restartRouter()
    } finally {
      await run()
    }
  }

  async function handleCopy() {
    const lines = [
      'gw client diagnostics',
      `server: ${client.session?.serverUrl ?? '-'}`,
      `device: ${client.snapshot?.device.name ?? '-'} / ${client.snapshot?.device.id ?? '-'}`,
      `user: ${client.snapshot?.user.email ?? '-'}`,
      `key: ${client.snapshot?.key.key_prefix ?? '-'}`,
      `router: ${client.localRouter?.baseUrl ?? client.routerError ?? '-'}`,
      `catalog: ${client.snapshot?.models.length ?? 0}`,
      '',
      ...checks.map((check) => `${check.ok ? 'OK' : 'FAIL'} ${check.label}: ${check.detail}${check.latencyMs !== undefined ? ` (${check.latencyMs}ms)` : ''}`),
      '',
      '不含 Key/token/对话内容',
    ]
    try {
      await navigator.clipboard.writeText(lines.join('\n'))
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  function handleForget() {
    client.forget()
    navigate('/welcome', { replace: true })
  }

  const failed = checks.filter((check) => !check.ok).length

  return (
    <>
      <PageHead
        title="诊断与恢复"
        desc="检查网关、本地路由与目录同步；诊断信息不包含 Key、token 或对话内容。"
        actions={
          <button className="btn" type="button" onClick={() => void run()} disabled={running}>
            {running ? '检测中…' : '重新检测'}
          </button>
        }
      />

      <div className={`note ${failed > 0 ? 'warn' : ''}`}>
        {running ? '正在检查…' : failed === 0 ? '所有检查通过' : `${failed} 项检查失败`}
      </div>

      <div className="card">
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>检查</th>
                <th>状态</th>
                <th>详情</th>
                <th>耗时</th>
              </tr>
            </thead>
            <tbody>
              {checks.map((check) => (
                <tr key={check.id}>
                  <td>{check.label}</td>
                  <td>
                    <Badge tone={check.ok ? 'ok' : 'bad'}>{check.ok ? '通过' : '失败'}</Badge>
                  </td>
                  <td style={{ wordBreak: 'break-all' }}>{check.detail}</td>
                  <td className="num">{check.latencyMs !== undefined ? `${check.latencyMs}ms` : '—'}</td>
                </tr>
              ))}
              {checks.length === 0 && (
                <tr>
                  <td colSpan={4} className="hint">
                    尚未运行检查。
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <div className="row" style={{ marginTop: 12, flexWrap: 'wrap' }}>
          <button className="btn" type="button" onClick={() => void handleRestartRouter()} disabled={running || !isDesktopRuntime()}>
            重启本地路由
          </button>
          <button className="btn" type="button" onClick={() => void handleCopy()} disabled={checks.length === 0}>
            {copied ? '已复制' : '复制诊断信息'}
          </button>
          <button className="btn" type="button" onClick={() => navigate('/settings')}>
            连接设置
          </button>
        </div>
      </div>

      <div className="card">
        <h3>恢复建议</h3>
        <ul className="hint" style={{ margin: '8px 0 0', paddingLeft: 18 }}>
          <li>healthz 失败：确认网关地址、VPN/网络与证书。</li>
          <li>readyz 失败：服务端数据库不可用，联系管理员。</li>
          <li>本地路由失败：点击重启；仍失败时重新配对或检查端口权限。</li>
          <li>目录为空：管理员尚未发布模型，或本组/用户不可见。</li>
        </ul>
      </div>

      <div className="card">
        <h3>清除本机配对</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          清除后需管理员重新生成配对码；诊断信息仍可复制。
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
