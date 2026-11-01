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
import { useI18n } from '../i18n/I18nProvider'

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
  const { tr } = useI18n()
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
      setActionError(
        err instanceof ApiError ? err.detail : tr('读取请求详情失败', 'Failed to load request details'),
      )
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
      setActionError(err instanceof Error ? err.message : tr('导出失败', 'Export failed'))
    }
  }

  return (
    <>
      <PageHead
        title={tr('请求日志', 'Request logs')}
        desc={tr(
          '请求元数据、状态与耗时；不含正文/headers/凭证。',
          'Request metadata, status, and latency. No body, headers, or credentials.',
        )}
        actions={
          <>
            <button className="btn" type="button" onClick={reload} disabled={loading}>
              {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
            </button>
            <button className="btn" type="button" onClick={() => void handleExport()}>
              {tr('导出 CSV', 'Export CSV')}
            </button>
          </>
        }
      />

      <div className="card">
        <form className="grid" onSubmit={applyFilters}>
          <div className="grid cols-3">
            <div className="field">
              <label htmlFor="log-from">{tr('开始时间', 'Start time')}</label>
              <input
                id="log-from"
                className="input"
                type="datetime-local"
                value={draft.from}
                onChange={(event) => setDraft({ ...draft, from: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-to">{tr('结束时间', 'End time')}</label>
              <input
                id="log-to"
                className="input"
                type="datetime-local"
                value={draft.to}
                onChange={(event) => setDraft({ ...draft, to: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-status">{tr('状态类', 'Status class')}</label>
              <select
                id="log-status"
                className="select"
                value={draft.status_class}
                onChange={(event) => setDraft({ ...draft, status_class: event.target.value })}
              >
                <option value="">{tr('全部', 'All')}</option>
                <option value="2xx">2xx</option>
                <option value="3xx">3xx</option>
                <option value="4xx">4xx</option>
                <option value="5xx">5xx</option>
              </select>
            </div>
          </div>
          <div className="grid cols-4">
            <div className="field">
              <label htmlFor="log-model">{tr('模型', 'Model')}</label>
              <input
                id="log-model"
                className="input"
                placeholder={tr('对外模型名', 'Public model name')}
                value={draft.model}
                onChange={(event) => setDraft({ ...draft, model: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-provider">{tr('供应商', 'Provider')}</label>
              <input
                id="log-provider"
                className="input"
                value={draft.provider}
                onChange={(event) => setDraft({ ...draft, provider: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-key-prefix">{tr('Key 前缀', 'Key prefix')}</label>
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
              {tr('查询', 'Search')}
            </button>
            <button className="btn" type="button" onClick={resetFilters}>
              {tr('重置', 'Reset')}
            </button>
            <span className="hint">
              {tr(
                '默认最近 7 天；limit 100，导出上限由服务端控制。',
                'Defaults to the last 7 days; limit 100. Export size is capped by the server.',
              )}
            </span>
          </div>
        </form>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取请求日志…', 'Loading request logs…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={tr('没有匹配的请求', 'No matching requests')}
          desc={tr(
            '调整时间范围/筛选条件；尚未产生流量时也会为空。',
            'Adjust the time range or filters. This list is also empty when no traffic has been recorded.',
          )}
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('时间', 'Time')}</th>
                <th>{tr('状态 / 耗时', 'Status / Latency')}</th>
                <th>{tr('模型', 'Model')}</th>
                <th>{tr('路径', 'Path')}</th>
                <th>{tr('供应商', 'Provider')}</th>
                <th>{tr('Key / 用户', 'Key / User')}</th>
                <th>{tr('请求 ID', 'Request ID')}</th>
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
                      {tr('详情', 'Details')}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {tr(
              `共 ${data.total} 条（最多显示 100 条，导出使用同筛选）`,
              `Total: ${data.total} rows (up to 100 shown; export uses the same filters)`,
            )}
          </div>
        </div>
      )}

      {detailLoading && (
        <Modal title={tr('请求详情', 'Request details')} onClose={() => setDetailLoading(false)}>
          <LoadingPanel title={tr('正在读取详情…', 'Loading details…')} />
        </Modal>
      )}

      {detail && (
        <Modal title={tr('请求详情', 'Request details')} onClose={() => setDetail(null)} wide>
          <dl className="kv">
            <dt>request_id</dt>
            <dd>
              <code className="kbd">{detail.request_id}</code>
            </dd>
            <dt>{tr('时间', 'Time')}</dt>
            <dd>{new Date(detail.created_at).toLocaleString('zh-CN')}</dd>
            <dt>{tr('路径 / 协议', 'Path / Protocol')}</dt>
            <dd>
              {detail.method} {detail.path} · {detail.protocol}
            </dd>
            <dt>{tr('模型', 'Model')}</dt>
            <dd>
              {detail.model_alias ?? '—'} → {detail.upstream_model ?? '—'}
            </dd>
            <dt>{tr('供应商 / adapter', 'Provider / adapter')}</dt>
            <dd>
              {detail.provider ?? '—'} · {detail.adapter ?? '—'}
            </dd>
            <dt>{tr('状态 / 耗时', 'Status / Latency')}</dt>
            <dd>
              {detail.status_code ?? '—'} · {detail.latency_ms}ms
              {detail.ttft_ms !== null ? ` · TTFT ${detail.ttft_ms}ms` : ''}
            </dd>
            <dt>attempt / fallback</dt>
            <dd>
              {detail.attempt_count} · {detail.fallback ? tr('是', 'Yes') : tr('否', 'No')}
            </dd>
            <dt>{tr('错误码', 'Error code')}</dt>
            <dd>{detail.error_code ?? '—'}</dd>
            <dt>{tr('用户 / 设备 / Key', 'User / Device / Key')}</dt>
            <dd>
              {detail.user_id ?? '—'} / {detail.device_id ?? '—'} / {detail.key_prefix ?? '—'}
            </dd>
            <dt>{tr('客户端 IP / UA', 'Client IP / UA')}</dt>
            <dd>
              {detail.client_ip ?? '—'}
              <div className="hint">{detail.user_agent || '—'}</div>
            </dd>
            <dt>usage</dt>
            <dd>
              {detail.usage
                ? tr(
                    `输入 ${detail.usage.input_tokens} · 输出 ${detail.usage.output_tokens} · ${detail.usage.cost} ${detail.usage.currency}（${detail.usage.usage_source}）`,
                    `Input ${detail.usage.input_tokens} · Output ${detail.usage.output_tokens} · ${detail.usage.cost} ${detail.usage.currency} (${detail.usage.usage_source})`,
                  )
                : '—'}
            </dd>
          </dl>
        </Modal>
      )}
    </>
  )
}
