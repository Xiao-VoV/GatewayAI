import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import {
  downloadCsv,
  getRequestDetail,
  listRequests,
  requestExportUrl,
} from '../api/observabilityApi'
import type { RequestFilters, RequestLogDetail, RequestLogItem } from '../api/observability'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { Modal } from '../components/Modal'

interface FilterDraft {
  from: string
  to: string
  status_class: string
  model: string
  provider: string
  key_prefix: string
  trace_id: string
}

const emptyFilters: FilterDraft = {
  from: '',
  to: '',
  status_class: '',
  model: '',
  provider: '',
  key_prefix: '',
  trace_id: '',
}

function toIso(value: string) {
  return value ? new Date(value).toISOString() : undefined
}

function statusBadge(item: RequestLogItem) {
  const code = item.status_code ?? 0
  if (item.error_code) return <Badge tone={code >= 500 ? 'bad' : 'warn'}>{item.error_code}</Badge>
  if (code >= 200 && code < 300) return <Badge tone="ok">{code}</Badge>
  if (code >= 400) return <Badge tone="bad">{code}</Badge>
  return <Badge>{code || '—'}</Badge>
}

export function LogsPage() {
  const [draft, setDraft] = useState<FilterDraft>(emptyFilters)
  const [filters, setFilters] = useState<FilterDraft>(emptyFilters)
  const [detail, setDetail] = useState<RequestLogDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const { data, loading, error, reload } = useAsync(
    () =>
      listRequests({
        from: toIso(filters.from),
        to: toIso(filters.to),
        status_class: filters.status_class || undefined,
        model: filters.model.trim() || undefined,
        provider: filters.provider.trim() || undefined,
        key_prefix: filters.key_prefix.trim() || undefined,
        trace_id: filters.trace_id.trim() || undefined,
        limit: 100,
      }),
    [filters],
  )

  function applyFilters(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFilters({ ...draft })
  }

  function resetFilters() {
    setDraft(emptyFilters)
    setFilters(emptyFilters)
  }

  async function openDetail(item: RequestLogItem) {
    setDetailLoading(true)
    setActionError(null)
    try {
      setDetail(await getRequestDetail(item.request_id))
    } catch (err) {
      setActionError(err instanceof ApiError ? err.detail : '读取请求详情失败')
    } finally {
      setDetailLoading(false)
    }
  }

  async function handleExport() {
    setActionError(null)
    try {
      await downloadCsv(
        requestExportUrl({
          from: toIso(filters.from),
          to: toIso(filters.to),
          status_class: filters.status_class || undefined,
          model: filters.model.trim() || undefined,
          provider: filters.provider.trim() || undefined,
          key_prefix: filters.key_prefix.trim() || undefined,
          trace_id: filters.trace_id.trim() || undefined,
        }),
        'requests.csv',
      )
    } catch (err) {
      setActionError(err instanceof Error ? err.message : '导出失败')
    }
  }

  return (
    <>
      <PageHead
        title="请求日志"
        desc="请求元数据、状态与耗时；不含正文/headers/凭证。"
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
          <div className="grid cols-3">
            <div className="field">
              <label htmlFor="log-from">开始时间</label>
              <input
                id="log-from"
                className="input"
                type="datetime-local"
                value={draft.from}
                onChange={(event) => setDraft({ ...draft, from: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-to">结束时间</label>
              <input
                id="log-to"
                className="input"
                type="datetime-local"
                value={draft.to}
                onChange={(event) => setDraft({ ...draft, to: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-status">状态类</label>
              <select
                id="log-status"
                className="select"
                value={draft.status_class}
                onChange={(event) => setDraft({ ...draft, status_class: event.target.value })}
              >
                <option value="">全部</option>
                <option value="2xx">2xx</option>
                <option value="3xx">3xx</option>
                <option value="4xx">4xx</option>
                <option value="5xx">5xx</option>
              </select>
            </div>
          </div>
          <div className="grid cols-4">
            <div className="field">
              <label htmlFor="log-model">模型</label>
              <input
                id="log-model"
                className="input"
                placeholder="对外模型名"
                value={draft.model}
                onChange={(event) => setDraft({ ...draft, model: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-provider">供应商</label>
              <input
                id="log-provider"
                className="input"
                value={draft.provider}
                onChange={(event) => setDraft({ ...draft, provider: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-key-prefix">Key 前缀</label>
              <input
                id="log-key-prefix"
                className="input"
                placeholder="sk-gw-abcd"
                value={draft.key_prefix}
                onChange={(event) => setDraft({ ...draft, key_prefix: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-trace">trace_id</label>
              <input
                id="log-trace"
                className="input"
                value={draft.trace_id}
                onChange={(event) => setDraft({ ...draft, trace_id: event.target.value })}
              />
            </div>
          </div>
          <div className="row">
            <button className="btn primary" type="submit">
              查询
            </button>
            <button className="btn" type="button" onClick={resetFilters}>
              重置
            </button>
            <span className="hint">默认最近 7 天；limit 100，导出上限由服务端控制。</span>
          </div>
        </form>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title="正在读取请求日志…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel title="没有匹配的请求" desc="调整时间范围/筛选条件；尚未产生流量时也会为空。" />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>时间</th>
                <th>状态 / 耗时</th>
                <th>模型</th>
                <th>路径</th>
                <th>供应商</th>
                <th>Key / 用户</th>
                <th>请求 ID</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {data.items.map((item) => (
                <tr key={item.id}>
                  <td>{new Date(item.created_at).toLocaleString('zh-CN')}</td>
                  <td>
                    {statusBadge(item)}
                    <div className="hint">
                      {item.latency_ms}ms{item.ttft_ms !== null ? ` · TTFT ${item.ttft_ms}ms` : ''}
                    </div>
                  </td>
                  <td>
                    {item.model_alias ?? '—'}
                    <div className="hint">{item.upstream_model ?? ''}</div>
                  </td>
                  <td>
                    <code className="kbd">{item.path}</code>
                    <div className="hint">
                      {item.protocol}
                      {item.fallback ? ' · fallback' : ''}
                    </div>
                  </td>
                  <td>{item.provider ?? '—'}</td>
                  <td>
                    {item.key_prefix ?? '—'}
                    <div className="hint">{item.user_id ? item.user_id.slice(0, 8) : 'anonymous'}</div>
                  </td>
                  <td>
                    <code className="kbd">{item.request_id.slice(0, 12)}…</code>
                  </td>
                  <td>
                    <button className="btn sm" type="button" onClick={() => void openDetail(item)}>
                      详情
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            共 {data.total} 条（最多显示 100 条，导出使用同筛选）
          </div>
        </div>
      )}

      {detailLoading && (
        <Modal title="请求详情" onClose={() => setDetailLoading(false)}>
          <LoadingPanel title="正在读取详情…" />
        </Modal>
      )}

      {detail && (
        <Modal title="请求详情" onClose={() => setDetail(null)} wide>
          <dl className="kv">
            <dt>request_id</dt>
            <dd>
              <code className="kbd">{detail.request_id}</code>
            </dd>
            <dt>时间</dt>
            <dd>{new Date(detail.created_at).toLocaleString('zh-CN')}</dd>
            <dt>路径 / 协议</dt>
            <dd>
              {detail.method} {detail.path} · {detail.protocol}
            </dd>
            <dt>模型</dt>
            <dd>
              {detail.model_alias ?? '—'} → {detail.upstream_model ?? '—'}
            </dd>
            <dt>供应商 / adapter</dt>
            <dd>
              {detail.provider ?? '—'} · {detail.adapter ?? '—'}
            </dd>
            <dt>状态 / 耗时</dt>
            <dd>
              {detail.status_code ?? '—'} · {detail.latency_ms}ms
              {detail.ttft_ms !== null ? ` · TTFT ${detail.ttft_ms}ms` : ''}
            </dd>
            <dt>attempt / fallback</dt>
            <dd>
              {detail.attempt_count} · {detail.fallback ? '是' : '否'}
            </dd>
            <dt>错误码</dt>
            <dd>{detail.error_code ?? '—'}</dd>
            <dt>用户 / 设备 / Key</dt>
            <dd>
              {detail.user_id ?? '—'} / {detail.device_id ?? '—'} / {detail.key_prefix ?? '—'}
            </dd>
            <dt>客户端 IP / UA</dt>
            <dd>
              {detail.client_ip ?? '—'}
              <div className="hint">{detail.user_agent || '—'}</div>
            </dd>
            <dt>usage</dt>
            <dd>
              {detail.usage
                ? `输入 ${detail.usage.input_tokens} · 输出 ${detail.usage.output_tokens} · ${detail.usage.cost} ${detail.usage.currency}（${detail.usage.usage_source}）`
                : '—'}
            </dd>
          </dl>
        </Modal>
      )}
    </>
  )
}
