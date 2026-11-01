import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import { auditExportUrl, downloadCsv, listAudit } from '../api/observabilityApi'
import type { AuditFilters, AuditItem } from '../api/observability'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { Modal } from '../components/Modal'

interface FilterDraft {
  from: string
  to: string
  action: string
  resource_type: string
  resource_id: string
  outcome: string
}

const emptyFilters: FilterDraft = {
  from: '',
  to: '',
  action: '',
  resource_type: '',
  resource_id: '',
  outcome: '',
}

function toIso(value: string) {
  return value ? new Date(value).toISOString() : undefined
}

function outcomeBadge(outcome: string) {
  if (outcome === 'success') return <Badge tone="ok">success</Badge>
  if (outcome === 'denied') return <Badge tone="warn">denied</Badge>
  return <Badge tone="bad">failure</Badge>
}

export function AuditPage() {
  const [draft, setDraft] = useState<FilterDraft>(emptyFilters)
  const [filters, setFilters] = useState<FilterDraft>(emptyFilters)
  const [detail, setDetail] = useState<AuditItem | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const { data, loading, error, reload } = useAsync(
    () =>
      listAudit({
        from: toIso(filters.from),
        to: toIso(filters.to),
        action: filters.action.trim() || undefined,
        resource_type: filters.resource_type.trim() || undefined,
        resource_id: filters.resource_id.trim() || undefined,
        outcome: filters.outcome || undefined,
        limit: 100,
      }),
    [filters],
  )

  function applyFilters(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFilters({ ...draft })
  }

  async function handleExport() {
    setActionError(null)
    try {
      await downloadCsv(
        auditExportUrl({
          from: toIso(filters.from),
          to: toIso(filters.to),
          action: filters.action.trim() || undefined,
          resource_type: filters.resource_type.trim() || undefined,
          resource_id: filters.resource_id.trim() || undefined,
          outcome: filters.outcome || undefined,
        }),
        'audit.csv',
      )
    } catch (err) {
      setActionError(err instanceof ApiError ? err.detail : err instanceof Error ? err.message : '导出失败')
    }
  }

  return (
    <>
      <PageHead
        title="审计"
        desc="管理员写操作审计；事件只追加，不含 headers/正文/凭证。"
        actions={
          <>
            <button className="btn" type="button" onClick={reload} disabled={loading}>
              {loading ? '刷新中…' : '刷新'}
            </button>
            <button className="btn" type="button" onClick={() => void handleExport()}>
              导出 CSV
            </button>
          </>
        }
      />

      <div className="card">
        <form className="grid" onSubmit={applyFilters}>
          <div className="grid cols-4">
            <div className="field">
              <label htmlFor="audit-from">开始时间</label>
              <input
                id="audit-from"
                className="input"
                type="datetime-local"
                value={draft.from}
                onChange={(event) => setDraft({ ...draft, from: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="audit-to">结束时间</label>
              <input
                id="audit-to"
                className="input"
                type="datetime-local"
                value={draft.to}
                onChange={(event) => setDraft({ ...draft, to: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="audit-outcome">结果</label>
              <select
                id="audit-outcome"
                className="select"
                value={draft.outcome}
                onChange={(event) => setDraft({ ...draft, outcome: event.target.value })}
              >
                <option value="">全部</option>
                <option value="success">success</option>
                <option value="failure">failure</option>
                <option value="denied">denied</option>
              </select>
            </div>
            <div className="field">
              <label htmlFor="audit-action">动作</label>
              <input
                id="audit-action"
                className="input"
                placeholder="access.key.revoked"
                value={draft.action}
                onChange={(event) => setDraft({ ...draft, action: event.target.value })}
              />
            </div>
          </div>
          <div className="grid cols-2">
            <div className="field">
              <label htmlFor="audit-resource-type">资源类型</label>
              <input
                id="audit-resource-type"
                className="input"
                placeholder="provider / model_group / key …"
                value={draft.resource_type}
                onChange={(event) => setDraft({ ...draft, resource_type: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="audit-resource-id">资源 ID</label>
              <input
                id="audit-resource-id"
                className="input"
                value={draft.resource_id}
                onChange={(event) => setDraft({ ...draft, resource_id: event.target.value })}
              />
            </div>
          </div>
          <div className="row">
            <button className="btn primary" type="submit">
              查询
            </button>
            <button
              className="btn"
              type="button"
              onClick={() => {
                setDraft(emptyFilters)
                setFilters(emptyFilters)
              }}
            >
              重置
            </button>
            <span className="hint">默认最近 7 天、最新 100 条；结果成败也记录 denied。</span>
          </div>
        </form>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title="正在读取审计…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel title="没有匹配的审计事件" desc="管理员写操作会记录；调整筛选或时间范围。" />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>时间</th>
                <th>操作者</th>
                <th>动作</th>
                <th>资源</th>
                <th>结果</th>
                <th>IP</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {data.items.map((item) => (
                <tr key={item.id}>
                  <td>{new Date(item.created_at).toLocaleString('zh-CN')}</td>
                  <td>
                    {item.actor_email ?? item.actor_type}
                    <div className="hint">{item.actor_type}</div>
                  </td>
                  <td>
                    <code className="kbd">{item.action}</code>
                  </td>
                  <td>
                    {item.resource_type}
                    <div className="hint">{item.resource_id ?? '—'}</div>
                  </td>
                  <td>{outcomeBadge(item.outcome)}</td>
                  <td>{item.client_ip ?? '—'}</td>
                  <td>
                    <button className="btn sm" type="button" onClick={() => setDetail(item)}>
                      详情
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            共 {data.total} 条；detail 已脱敏。
          </div>
        </div>
      )}

      {detail && (
        <Modal title="审计详情" onClose={() => setDetail(null)} wide>
          <dl className="kv">
            <dt>事件 ID</dt>
            <dd>
              <code className="kbd">{detail.id}</code>
            </dd>
            <dt>时间</dt>
            <dd>{new Date(detail.created_at).toLocaleString('zh-CN')}</dd>
            <dt>操作者</dt>
            <dd>
              {detail.actor_email ?? '—'} · {detail.actor_type} · {detail.actor_id ?? '—'}
            </dd>
            <dt>动作</dt>
            <dd>
              <code className="kbd">{detail.action}</code>
            </dd>
            <dt>资源</dt>
            <dd>
              {detail.resource_type} / {detail.resource_id ?? '—'}
            </dd>
            <dt>结果</dt>
            <dd>{detail.outcome}</dd>
            <dt>request_id</dt>
            <dd>{detail.request_id ?? '—'}</dd>
            <dt>IP / UA</dt>
            <dd>
              {detail.client_ip ?? '—'}
              <div className="hint">{detail.user_agent}</div>
            </dd>
            <dt>detail</dt>
            <dd>
              <pre style={{ whiteSpace: 'pre-wrap', fontSize: 11.5 }}>{JSON.stringify(detail.detail, null, 2)}</pre>
            </dd>
          </dl>
        </Modal>
      )}
    </>
  )
}
