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
      <div className="bars" role="img" aria-label={tr('最近 7 天请求趋势', 'Request trend over the last 7 days')}>
        {trend.map((day) => {
          const height = day.requests === 0 ? 6 : Math.max(8, Math.round((day.requests / max) * 100))
          return (
            <i
              key={day.day}
              className={day.day === trend[trend.length - 1]?.day ? 'hi' : ''}
              style={{ height: `${height}%` }}
              title={`${day.day} · ${formatNumber(day.requests)} ${tr('请求', 'requests')}`}
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
    success: tr('成功', 'Success'),
    upstream_error: tr('上游错误', 'Upstream error'),
    timeout: tr('超时', 'Timeout'),
    gateway_error: tr('网关错误', 'Gateway error'),
    client_disconnect: tr('客户端断开', 'Client disconnected'),
    rejected: tr('已拒绝', 'Rejected'),
  }

  const summaryDesc = tr(
    '今天的请求、花费、错误率；数据来自 metering。',
    'Requests, spend and error rate for today, sourced from metering.',
  )

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  if (loading && !data) {
    return (
      <>
        <PageHead title={tr('总览', 'Overview')} desc={summaryDesc} actions={refresh} />
        <LoadingPanel title={tr('正在读取总览…', 'Loading overview…')} />
      </>
    )
  }

  if (error) {
    return (
      <>
        <PageHead title={tr('总览', 'Overview')} desc={summaryDesc} actions={refresh} />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }

  if (!data) {
    return (
      <>
        <PageHead title={tr('总览', 'Overview')} actions={refresh} />
        <LoadingPanel />
      </>
    )
  }

  if (data.totals.requests === 0) {
    return (
      <>
        <PageHead title={tr('总览', 'Overview')} desc={summaryDesc} actions={refresh} />
        <EmptyPanel
          title={tr('还没有请求数据', 'No request data yet')}
          desc={tr(
            '先配置供应商、模型组与发布，并生成设备配对码；完成第一次推理后，这里会出现请求、花费和错误趋势。',
            'Configure a provider, model group and release, then generate a device pairing code. After the first inference, requests, spend and error trends will appear here.',
          )}
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
        title={tr('总览', 'Overview')}
        desc={`${tr(`最近 ${data.days} 天；`, `Last ${data.days} days; `)}${data.timezone} · ${tr(
          '更新于',
          'Updated',
        )} ${new Date(data.generated_at).toLocaleString('zh-CN')}`}
        actions={refresh}
      />

      <div className="grid cols-4">
        <div className="card stat">
          <div className="sub">{tr('今日请求', 'Requests today')}</div>
          <div className="num">{formatNumber(data.today.requests)}</div>
          <div className="metric-change">
            {tr('成功', 'Success')} {formatNumber(data.today.success_requests)} · {tr('拒绝', 'Rejected')}{' '}
            {formatNumber(data.today.rejected_requests)}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">{tr('今日花费', 'Spend today')}</div>
          <div className="num">{formatCost(data.today.cost)}</div>
          <div className="metric-change">
            {tr('币种', 'Currency')} {data.currency}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">{tr('今日错误率', 'Error rate today')}</div>
          <div className="num">{formatErrorRate(errorRate)}</div>
          <div className="metric-change">
            {tr('错误', 'Errors')} {formatNumber(data.today.error_requests)} · {tr('断开', 'Disconnects')}{' '}
            {formatNumber(data.today.client_disconnect_requests)}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">{tr(`最近 ${data.days} 天请求`, `Requests in the last ${data.days} days`)}</div>
          <div className="num">{formatNumber(data.totals.requests)}</div>
          <div className="metric-change">
            {tr('花费', 'Spend')} {formatCost(data.totals.cost)}
          </div>
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{tr('请求趋势', 'Request trend')}</h3>
            <span className="sub">{tr(`最近 ${data.days} 天`, `Last ${data.days} days`)}</span>
          </div>
          {data.trend.length === 0 ? (
            <div className="chart-empty">{tr('暂无趋势数据', 'No trend data yet')}</div>
          ) : (
            <TrendChart trend={data.trend} />
          )}
        </div>
        <div className="card">
          <div className="row between">
            <h3>{tr('今日状态', 'Status today')}</h3>
            <span className="sub">{tr('按请求结果', 'By request outcome')}</span>
          </div>
          {statusEntries.length === 0 ? (
            <div className="chart-empty">{tr('今日暂无请求', 'No requests today')}</div>
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
            <h3>{tr('Token 与成本', 'Tokens & cost')}</h3>
            <span className="sub">{tr('今日', 'Today')}</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <div className="status-row">
              <span className="sub">{tr('输入 tokens', 'Input tokens')}</span>
              <span style={{ marginLeft: 'auto' }}>{formatNumber(data.today.input_tokens)}</span>
            </div>
            <div className="status-row">
              <span className="sub">{tr('输出 tokens', 'Output tokens')}</span>
              <span style={{ marginLeft: 'auto' }}>{formatNumber(data.today.output_tokens)}</span>
            </div>
            <div className="status-row">
              <span className="sub">{tr('今日成本', 'Cost today')}</span>
              <span style={{ marginLeft: 'auto' }}>{formatCost(data.today.cost)}</span>
            </div>
          </div>
        </div>
        <div className="card">
          <div className="row between">
            <h3>{tr('合计', 'Totals')}</h3>
            <span className="sub">{tr(`最近 ${data.days} 天`, `Last ${data.days} days`)}</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <div className="status-row">
              <span className="sub">
                {tr('成功', 'Success')} / {tr('错误', 'Errors')}
              </span>
              <span style={{ marginLeft: 'auto' }}>
                {formatNumber(data.totals.success_requests)} / {formatNumber(data.totals.error_requests)}
              </span>
            </div>
            <div className="status-row">
              <span className="sub">{tr('输入 / 输出 tokens', 'Input / output tokens')}</span>
              <span style={{ marginLeft: 'auto' }}>
                {formatNumber(data.totals.input_tokens)} / {formatNumber(data.totals.output_tokens)}
              </span>
            </div>
            <div className="status-row">
              <span className="sub">{tr('总成本', 'Total cost')}</span>
              <span style={{ marginLeft: 'auto' }}>{formatCost(data.totals.cost)}</span>
            </div>
          </div>
        </div>
      </div>
    </>
  )
}
