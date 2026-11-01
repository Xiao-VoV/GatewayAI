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
        err instanceof ApiError ? err.detail : tr("failed_to_load_request_details"),
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
      setActionError(err instanceof Error ? err.message : tr("export_failed"))
    }
  }

  return (
    <>
      <PageHead
        title={tr("request_logs")}
        desc={tr("request_metadata_status_and_latency_no_body_headers_or_c")}
        actions={
          <>
            <button className="btn" type="button" onClick={reload} disabled={loading}>
              {loading ? tr("refreshing") : tr("refresh")}
            </button>
            <button className="btn" type="button" onClick={() => void handleExport()}>
              {tr("export_csv")}
            </button>
          </>
        }
      />

      <div className="card">
        <form className="grid" onSubmit={applyFilters}>
          <div className="grid cols-3">
            <div className="field">
              <label htmlFor="log-from">{tr("start_time")}</label>
              <input
                id="log-from"
                className="input"
                type="datetime-local"
                value={draft.from}
                onChange={(event) => setDraft({ ...draft, from: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-to">{tr("end_time")}</label>
              <input
                id="log-to"
                className="input"
                type="datetime-local"
                value={draft.to}
                onChange={(event) => setDraft({ ...draft, to: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-status">{tr("status_class")}</label>
              <select
                id="log-status"
                className="select"
                value={draft.status_class}
                onChange={(event) => setDraft({ ...draft, status_class: event.target.value })}
              >
                <option value="">{tr("all")}</option>
                <option value="2xx">2xx</option>
                <option value="3xx">3xx</option>
                <option value="4xx">4xx</option>
                <option value="5xx">5xx</option>
              </select>
            </div>
          </div>
          <div className="grid cols-4">
            <div className="field">
              <label htmlFor="log-model">{tr("model")}</label>
              <input
                id="log-model"
                className="input"
                placeholder={tr("public_model_name")}
                value={draft.model}
                onChange={(event) => setDraft({ ...draft, model: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-provider">{tr("provider")}</label>
              <input
                id="log-provider"
                className="input"
                value={draft.provider}
                onChange={(event) => setDraft({ ...draft, provider: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="log-key-prefix">{tr("key_prefix")}</label>
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
              {tr("search")}
            </button>
            <button className="btn" type="button" onClick={resetFilters}>
              {tr("reset")}
            </button>
            <span className="hint">
              {tr("defaults_to_the_last_7_days_limit_100_export_size_is_cap")}
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
        <LoadingPanel title={tr("loading_request_logs")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={tr("no_matching_requests")}
          desc={tr("adjust_the_time_range_or_filters_this_list_is_also_empty")}
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("time")}</th>
                <th>{tr("status_latency")}</th>
                <th>{tr("model")}</th>
                <th>{tr("path")}</th>
                <th>{tr("provider")}</th>
                <th>{tr("key_user")}</th>
                <th>{tr("request_id")}</th>
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
                      {tr("details")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {tr("total_rows_up_to_100_shown_export_uses_the_same_filters", { total: data.total })}
          </div>
        </div>
      )}

      {detailLoading && (
        <Modal title={tr("request_details")} onClose={() => setDetailLoading(false)}>
          <LoadingPanel title={tr("loading_details")} />
        </Modal>
      )}

      {detail && (
        <Modal title={tr("request_details")} onClose={() => setDetail(null)} wide>
          <dl className="kv">
            <dt>request_id</dt>
            <dd>
              <code className="kbd">{detail.request_id}</code>
            </dd>
            <dt>{tr("time")}</dt>
            <dd>{new Date(detail.created_at).toLocaleString('zh-CN')}</dd>
            <dt>{tr("path_protocol")}</dt>
            <dd>
              {detail.method} {detail.path} · {detail.protocol}
            </dd>
            <dt>{tr("model")}</dt>
            <dd>
              {detail.model_alias ?? '—'} → {detail.upstream_model ?? '—'}
            </dd>
            <dt>{tr("provider_adapter")}</dt>
            <dd>
              {detail.provider ?? '—'} · {detail.adapter ?? '—'}
            </dd>
            <dt>{tr("status_latency")}</dt>
            <dd>
              {detail.status_code ?? '—'} · {detail.latency_ms}ms
              {detail.ttft_ms !== null ? ` · TTFT ${detail.ttft_ms}ms` : ''}
            </dd>
            <dt>attempt / fallback</dt>
            <dd>
              {detail.attempt_count} · {detail.fallback ? tr("yes") : tr("no")}
            </dd>
            <dt>{tr("error_code")}</dt>
            <dd>{detail.error_code ?? '—'}</dd>
            <dt>{tr("user_device_key")}</dt>
            <dd>
              {detail.user_id ?? '—'} / {detail.device_id ?? '—'} / {detail.key_prefix ?? '—'}
            </dd>
            <dt>{tr("client_ip_ua")}</dt>
            <dd>
              {detail.client_ip ?? '—'}
              <div className="hint">{detail.user_agent || '—'}</div>
            </dd>
            <dt>usage</dt>
            <dd>
              {detail.usage
                ? tr("input_output", { input_tokens: detail.usage.input_tokens, output_tokens: detail.usage.output_tokens, cost: detail.usage.cost, currency: detail.usage.currency, usage_source: detail.usage.usage_source })
                : '—'}
            </dd>
          </dl>
        </Modal>
      )}
    </>
  )
}
