import { useCallback, useEffect, useState } from 'react'
import { useClient } from '../auth/ClientProvider'
import { isDesktopRuntime } from '../platform/secureStore'
import {
  agentConfigStatus,
  applyAgentConfig,
  previewAgentConfig,
  rollbackAgentConfig,
  scanAgents,
} from '../platform/nativeAgents'
import type { AgentApplyResult, AgentConfigStatus, AgentInfo, AgentPlan, AgentScan } from '../platform/nativeAgents'
import { Badge, PageHead } from '../components/ui'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { useUiPreferences } from '../ui/preferences'

const statusMeta = (en: boolean): Record<AgentInfo['status'], { label: string; tone: '' | 'ok' | 'warn' | 'bad' }> => ({
  configured: { label: en ? 'Configured' : '已配置', tone: 'ok' },
  available: { label: en ? 'Not configured' : '未配置', tone: 'warn' },
  missing: { label: en ? 'Not installed' : '未安装', tone: '' },
})

export function ToolsPage() {
  const { session, snapshot, localRouter, routerError } = useClient()
  const { locale } = useUiPreferences()
  const en = locale === 'en-US'
  const desktop = isDesktopRuntime()
  const [scan, setScan] = useState<AgentScan | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [selected, setSelected] = useState<AgentInfo | null>(null)
  const [plan, setPlan] = useState<AgentPlan | null>(null)
  const [status, setStatus] = useState<AgentConfigStatus | null>(null)
  const [applyResult, setApplyResult] = useState<AgentApplyResult | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  const defaultModelFor = useCallback(
    (agent: AgentInfo) =>
      snapshot?.tools.find((tool) => tool.tool_key === agent.id)?.default_model ??
      snapshot?.tools[0]?.default_model,
    [snapshot],
  )

  const runScan = useCallback(async () => {
    if (!desktop) return
    setLoading(true)
    setError(null)
    try {
      setScan(await scanAgents())
    } catch (err) {
      setError(err instanceof Error ? err.message : en ? 'Scan failed' : '扫描失败')
    } finally {
      setLoading(false)
    }
  }, [desktop])

  useEffect(() => {
    void runScan()
  }, [runScan])

  async function handleSelect(agent: AgentInfo) {
    setSelected(agent)
    setPlan(null)
    setStatus(null)
    setApplyResult(null)
    setActionError(null)
    if (!desktop || !agent.supportsWrite || agent.status === 'missing') return
    const baseUrl = localRouter?.baseUrl ?? session?.serverUrl ?? ''
    if (!baseUrl) return
    setBusy('load')
    try {
      const [nextPlan, nextStatus] = await Promise.all([
        previewAgentConfig(agent.id, baseUrl, defaultModelFor(agent)),
        agentConfigStatus(agent.id),
      ])
      setPlan(nextPlan)
      setStatus(nextStatus)
    } catch (err) {
      setActionError(err instanceof Error ? err.message : en ? 'Failed to build config plan' : '生成配置计划失败')
    } finally {
      setBusy(null)
    }
  }

  async function handleApply() {
    if (!selected || !localRouter) return
    setBusy('apply')
    setActionError(null)
    try {
      const result = await applyAgentConfig(selected.id, localRouter.baseUrl, defaultModelFor(selected))
      setApplyResult(result)
      setStatus(await agentConfigStatus(selected.id))
      await runScan()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : en ? 'Failed to write config' : '写入配置失败')
    } finally {
      setBusy(null)
    }
  }

  async function handleRollback() {
    if (!selected) return
    setBusy('rollback')
    setActionError(null)
    try {
      await rollbackAgentConfig(selected.id)
      setApplyResult(null)
      setStatus(await agentConfigStatus(selected.id))
      await runScan()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : en ? 'Failed to roll back config' : '回滚配置失败')
    } finally {
      setBusy(null)
    }
  }

  if (!desktop) {
    return (
      <>
        <PageHead title={en ? 'AI Tools' : 'AI 工具'} desc={en ? 'Scan locally installed AI tools and prepare them for access.' : '扫描本机已安装的 AI 工具并准备接入。'} />
        <EmptyPanel
          title={en ? 'Tool management requires the desktop app' : '工具管理需要桌面版'}
          desc={en ? 'Browser preview cannot read local files or commands. Start the Tauri desktop app with `pnpm --filter @gw/client desktop:dev` to scan and configure tools.' : '浏览器预览无法读取本机文件与命令。使用 `pnpm --filter @gw/client desktop:dev` 启动 Tauri 桌面版后即可扫描和配置本机工具。'}
        />
      </>
    )
  }

  return (
    <>
      <PageHead
        title={en ? 'AI Tools' : 'AI 工具'}
        desc={en ? 'Scanning and configuration are done by the Rust backend; this page only renders capabilities and plans.' : 'Rust 后端扫描/配置；页面只展示后端返回的能力与计划。'}
        actions={
          <button className="btn" type="button" onClick={() => void runScan()} disabled={loading}>
            {loading ? (en ? 'Scanning…' : '扫描中…') : en ? 'Rescan' : '重新扫描'}
          </button>
        }
      />

      {routerError && <div className="note warn">{en ? 'Local router is not running in browser preview. Config writes require the desktop app.' : `本地路由未启动：${routerError}。配置写入需要本地路由运行。`}</div>}
      {localRouter && (
        <div className="note">
          {en ? 'Local router running: ' : '本地路由运行中：'}<code className="kbd">{localRouter.baseUrl}</code>{en ? '. Written configs point here; real keys are injected by the router.' : '。写入配置会指向该地址，真实 Key 由路由注入。'}
        </div>
      )}
      {error && <ErrorPanel message={error} onRetry={() => void runScan()} />}

      {loading && !scan ? (
        <LoadingPanel title={en ? 'Scanning local AI tools…' : '正在扫描本机 AI 工具…'} />
      ) : !scan ? null : (
        <>
          {scan.warnings.length > 0 && (
            <div className="note warn">
              {scan.warnings.map((warning) => (
                <div key={warning}>{warning}</div>
              ))}
            </div>
          )}
          <div className="table-wrap">
            <table className="table dense">
              <thead>
                <tr>
                  <th>工具</th>
                  <th>状态</th>
                  <th>命令 / 格式</th>
                  <th>配置路径</th>
                  <th>检测到的端点</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {scan.agents.map((agent) => {
                  const meta = statusMeta(en)[agent.status]
                  return (
                    <tr key={agent.id} className={selected?.id === agent.id ? 'selected' : ''}>
                      <td>
                        <strong>{agent.name}</strong>
                        <div className="hint">
                          {agent.id}
                          {agent.supportsWrite ? (en ? ' · writable' : ' · 可写') : en ? ' · read-only' : ' · 只读'}
                        </div>
                      </td>
                      <td>
                        <Badge tone={meta.tone}>{meta.label}</Badge>
                      </td>
                      <td>
                        {agent.command ? <code className="kbd">{agent.command}</code> : '—'}
                        <div className="hint">{agent.configFormat}</div>
                      </td>
                      <td>
                        <span className="hint" style={{ wordBreak: 'break-all' }}>
                          {agent.configPath ?? '—'}
                        </span>
                        <div className="hint">{agent.configExists ? (en ? 'Config found' : '已发现配置') : en ? 'No config file' : '未发现配置文件'}</div>
                      </td>
                      <td>{agent.endpoint ? <code className="kbd">{agent.endpoint}</code> : <span className="hint">{en ? 'Not configured' : '未配置'}</span>}</td>
                      <td>
                        <button className="btn sm" type="button" onClick={() => void handleSelect(agent)}>
                          {en ? 'View / configure' : '查看 / 配置'}
                        </button>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>

          {selected && (
            <div className="card">
              <div className="row between">
                <h3>{selected.name}</h3>
                <Badge tone={selected.supportsWrite ? 'accent' : 'warn'}>
                  {selected.supportsWrite ? (en ? 'Auto-write supported' : '支持自动写入') : en ? 'Read-only' : '只读检测'}
                </Badge>
              </div>
              {selected.notes.length > 0 && (
                <ul className="hint" style={{ margin: '8px 0', paddingLeft: 18 }}>
                  {selected.notes.map((note) => (
                    <li key={note}>{note}</li>
                  ))}
                </ul>
              )}
              {!selected.supportsWrite && (
                <p className="hint">
                  {en ? 'This tool is detection/preview only. Add a Rust AgentAdapter and register it to enable safe writes; the UI needs no changes.' : '该工具目前仅做发现/预览；需要安全写入方案时，在 Rust 新增 AgentAdapter 并在 registry 注册，UI 无需改动。'}
                </p>
              )}
              {selected.status === 'missing' && <p className="hint">{en ? 'Not detected on this device. Install it and rescan.' : '本机未检测到该工具，安装后重新扫描。'}</p>}
              {busy === 'load' && <LoadingPanel title={en ? 'Loading backend config status…' : '正在读取后端配置状态…'} />}
              {actionError && <div className="form-error">{actionError}</div>}
              {selected.supportsWrite && selected.status !== 'missing' && plan && (
                <>
                  <p className="hint" style={{ marginTop: 4 }}>
                    {en ? 'Target' : '目标'}：{plan.targetPath ?? '—'} · {en ? 'format' : '格式'} {plan.format}
                  </p>
                  <div className="table-wrap" style={{ marginTop: 8 }}>
                    <table className="table dense">
                      <thead>
                        <tr>
                          <th>{en ? 'Backend plan key' : '后端计划键'}</th>
                          <th>{en ? 'Value' : '值'}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {plan.changes.map((change) => (
                          <tr key={change.key}>
                            <td>
                              <code className="kbd">{change.key}</code>
                            </td>
                            <td>
                              <code className="kbd">{change.value}</code>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  {plan.warnings.map((warning) => (
                    <div className="note warn" key={warning} style={{ marginTop: 8 }}>
                      {warning}
                    </div>
                  ))}
                  <div className="row" style={{ marginTop: 10 }}>
                    <button
                      className="btn primary"
                      type="button"
                      onClick={() => void handleApply()}
                      disabled={!localRouter || busy !== null}
                    >
                      {busy === 'apply' ? (en ? 'Writing…' : '写入中…') : status?.applied ? (en ? 'Rewrite config' : '重新写入配置') : en ? 'Write config' : '写入配置'}
                    </button>
                    {status?.applied && (
                      <button className="btn danger" type="button" onClick={() => void handleRollback()} disabled={busy !== null}>
                        {busy === 'rollback' ? (en ? 'Rolling back…' : '回滚中…') : en ? 'Roll back' : '回滚到写入前'}
                      </button>
                    )}
                    <span className="hint">
                      {!localRouter ? (en ? 'Local router is required' : '需要本地路由运行') : en ? `Written and backed up by Rust; status: ${status?.applied ? 'applied' : 'not applied'}` : `配置由 Rust 写入并备份；当前状态：${status?.applied ? '已写入' : '未写入'}`}
                    </span>
                  </div>
                  {applyResult && (
                    <div className="note" style={{ marginTop: 8 }}>
                      {en ? `Applied ${applyResult.applied.length} keys` : `已写入 ${applyResult.applied.length} 个键`}
                      {applyResult.backupPath ? (en ? ` · backup ${applyResult.backupPath}` : ` · 备份 ${applyResult.backupPath}`) : en ? ' · original file did not exist; rollback will delete the new file' : ' · 原文件不存在，回滚将删除新文件'}
                    </div>
                  )}
                </>
              )}
            </div>
          )}
        </>
      )}
    </>
  )
}
