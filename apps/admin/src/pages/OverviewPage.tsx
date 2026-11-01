import { getOverview } from '../api/overview'
import { useAsync } from '../api/useAsync'
import { ErrorPanel, EmptyPanel, LoadingPanel } from '../components/ScreenState'
import { Badge, PageHead } from '../components/ui'
import { useI18n } from '../i18n/I18nProvider'

const loadOverview = () => getOverview(7)

const numberFormat = new Intl.NumberFormat('zh-CN')
const percentFormat = new Intl.NumberFormat('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 2 })

function formatNumber(value: number) {
  return numberFormat.format(value)
}

function formatCost(value: string) {
  const amount = Number(value)
  if (!Number.isFinite(amount)) return value
  return `¥${amount.toLocaleString('zh-CN', { minimumFractionDigits: 2, maximumFractionDigits: 6 })}`
}

function formatErrorRate(rate: number) {
  return `${percentFormat.format(rate * 100)}%`
}

function TrendChart({ trend }: { trend: { day: string; requests: number }[] }) {
  const { tr } = useI18n()
  const max = Math.max(1, ...trend.map((day) => day.requests))
  return (
    <div>
      <div className="bars" role="img" aria-label={tr("request_trend_over_the_last_7_days")}>
        {trend.map((day) => {
          const height = day.requests === 0 ? 6 : Math.max(8, Math.round((day.requests / max) * 100))
          return (
            <i
              key={day.day}
              className={day.day === trend[trend.length - 1]?.day ? 'hi' : ''}
              style={{ height: `${height}%` }}
              title={`${day.day} · ${formatNumber(day.requests)} ${tr("requests")}`}
            />
          )
        })}
      </div>
      <div className="bar-labels">
        {trend.map((day) => (
          <span className="bar-date" key={day.day}>
            {day.day.slice(5)}
          </span>
        ))}
      </div>
    </div>
  )
}

export function OverviewPage() {
  const { data, loading, error, reload } = useAsync(loadOverview)
  const { tr } = useI18n()

  const statusLabels: Record<string, string> = {
    success: tr("success"),
    upstream_error: tr("upstream_error"),
    timeout: tr("timeout"),
    gateway_error: tr("gateway_error"),
    client_disconnect: tr("client_disconnected"),
    rejected: tr("rejected_2"),
  }

  const summaryDesc = tr("requests_spend_and_error_rate_for_today_sourced_from_met")

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  if (loading && !data) {
    return (
      <>
        <PageHead title={tr("overview")} desc={summaryDesc} actions={refresh} />
        <LoadingPanel title={tr("loading_overview")} />
      </>
    )
  }

  if (error) {
    return (
      <>
        <PageHead title={tr("overview")} desc={summaryDesc} actions={refresh} />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }

  if (!data) {
    return (
      <>
        <PageHead title={tr("overview")} actions={refresh} />
        <LoadingPanel />
      </>
    )
  }

  if (data.totals.requests === 0) {
    return (
      <>
        <PageHead title={tr("overview")} desc={summaryDesc} actions={refresh} />
        <EmptyPanel
          title={tr("no_request_data_yet")}
          desc={tr("configure_a_provider_model_group_and_release_then_genera")}
        />
      </>
    )
  }

  const errorRate =
    data.today.error_rate ?? (data.today.requests > 0 ? data.today.error_requests / data.today.requests : 0)
  const statusEntries = Object.entries(data.today.status_breakdown).filter(([, count]) => count > 0)

  return (
    <>
      <PageHead
        title={tr("overview")}
        desc={`${tr("last_days_2", { days: data.days })}${data.timezone} · ${tr("updated_2")} ${new Date(data.generated_at).toLocaleString('zh-CN')}`}
        actions={refresh}
      />

      <div className="grid cols-4">
        <div className="card stat">
          <div className="sub">{tr("requests_today")}</div>
          <div className="num">{formatNumber(data.today.requests)}</div>
          <div className="metric-change">
            {tr("success")} {formatNumber(data.today.success_requests)} · {tr("rejected")}{' '}
            {formatNumber(data.today.rejected_requests)}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">{tr("spend_today")}</div>
          <div className="num">{formatCost(data.today.cost)}</div>
          <div className="metric-change">
            {tr("currency")} {data.currency}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">{tr("error_rate_today")}</div>
          <div className="num">{formatErrorRate(errorRate)}</div>
          <div className="metric-change">
            {tr("errors")} {formatNumber(data.today.error_requests)} · {tr("disconnects")}{' '}
            {formatNumber(data.today.client_disconnect_requests)}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">{tr("requests_in_the_last_days", { days: data.days })}</div>
          <div className="num">{formatNumber(data.totals.requests)}</div>
          <div className="metric-change">
            {tr("spend")} {formatCost(data.totals.cost)}
          </div>
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{tr("request_trend")}</h3>
            <span className="sub">{tr("last_days", { days: data.days })}</span>
          </div>
          {data.trend.length === 0 ? (
            <div className="chart-empty">{tr("no_trend_data_yet")}</div>
          ) : (
            <TrendChart trend={data.trend} />
          )}
        </div>
        <div className="card">
          <div className="row between">
            <h3>{tr("status_today")}</h3>
            <span className="sub">{tr("by_request_outcome")}</span>
          </div>
          {statusEntries.length === 0 ? (
            <div className="chart-empty">{tr("no_requests_today")}</div>
          ) : (
            <div style={{ marginTop: 8 }}>
              {statusEntries.map(([status, count]) => (
                <div className="status-row" key={status}>
                  <Badge tone={status === 'success' ? 'ok' : status === 'rejected' ? 'warn' : 'bad'}>
                    {statusLabels[status] ?? status}
                  </Badge>
                  <span className="num" style={{ marginLeft: 'auto', fontVariantNumeric: 'tabular-nums' }}>
                    {formatNumber(count)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{tr("tokens_cost")}</h3>
            <span className="sub">{tr("today")}</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <div className="status-row">
              <span className="sub">{tr("input_tokens")}</span>
              <span style={{ marginLeft: 'auto' }}>{formatNumber(data.today.input_tokens)}</span>
            </div>
            <div className="status-row">
              <span className="sub">{tr("output_tokens")}</span>
              <span style={{ marginLeft: 'auto' }}>{formatNumber(data.today.output_tokens)}</span>
            </div>
            <div className="status-row">
              <span className="sub">{tr("cost_today")}</span>
              <span style={{ marginLeft: 'auto' }}>{formatCost(data.today.cost)}</span>
            </div>
          </div>
        </div>
        <div className="card">
          <div className="row between">
            <h3>{tr("totals")}</h3>
            <span className="sub">{tr("last_days", { days: data.days })}</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <div className="status-row">
              <span className="sub">
                {tr("success")} / {tr("errors")}
              </span>
              <span style={{ marginLeft: 'auto' }}>
                {formatNumber(data.totals.success_requests)} / {formatNumber(data.totals.error_requests)}
              </span>
            </div>
            <div className="status-row">
              <span className="sub">{tr("input_output_tokens")}</span>
              <span style={{ marginLeft: 'auto' }}>
                {formatNumber(data.totals.input_tokens)} / {formatNumber(data.totals.output_tokens)}
              </span>
            </div>
            <div className="status-row">
              <span className="sub">{tr("total_cost")}</span>
              <span style={{ marginLeft: 'auto' }}>{formatCost(data.totals.cost)}</span>
            </div>
          </div>
        </div>
      </div>
    </>
  )
}
