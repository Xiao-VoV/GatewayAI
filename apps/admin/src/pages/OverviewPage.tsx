import { getOverview } from '../api/overview'
import { useAsync } from '../api/useAsync'
import { ErrorPanel, EmptyPanel, LoadingPanel } from '../components/ScreenState'
import { Badge, PageHead } from '../components/ui'

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

const statusLabels: Record<string, string> = {
  success: '成功',
  upstream_error: '上游错误',
  timeout: '超时',
  gateway_error: '网关错误',
  client_disconnect: '客户端断开',
  rejected: '已拒绝',
}

function TrendChart({ trend }: { trend: { day: string; requests: number }[] }) {
  const max = Math.max(1, ...trend.map((day) => day.requests))
  return (
    <div>
      <div className="bars" role="img" aria-label="最近 7 天请求趋势">
        {trend.map((day) => {
          const height = day.requests === 0 ? 6 : Math.max(8, Math.round((day.requests / max) * 100))
          return (
            <i
              key={day.day}
              className={day.day === trend[trend.length - 1]?.day ? 'hi' : ''}
              style={{ height: `${height}%` }}
              title={`${day.day} · ${formatNumber(day.requests)} 请求`}
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

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? '刷新中…' : '刷新'}
    </button>
  )

  if (loading && !data) {
    return (
      <>
        <PageHead title="总览" desc="今天的请求、花费、错误率；数据来自 metering。" actions={refresh} />
        <LoadingPanel title="正在读取总览…" />
      </>
    )
  }

  if (error) {
    return (
      <>
        <PageHead title="总览" desc="今天的请求、花费、错误率；数据来自 metering。" actions={refresh} />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }

  if (!data) {
    return (
      <>
        <PageHead title="总览" actions={refresh} />
        <LoadingPanel />
      </>
    )
  }

  if (data.totals.requests === 0) {
    return (
      <>
        <PageHead
          title="总览"
          desc="今天的请求、花费、错误率；数据来自 metering。"
          actions={refresh}
        />
        <EmptyPanel
          title="还没有请求数据"
          desc="先配置供应商、模型组与发布，并生成设备配对码；完成第一次推理后，这里会出现请求、花费和错误趋势。"
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
        title="总览"
        desc={`最近 ${data.days} 天；${data.timezone} · 更新于 ${new Date(data.generated_at).toLocaleString('zh-CN')}`}
        actions={refresh}
      />

      <div className="grid cols-4">
        <div className="card stat">
          <div className="sub">今日请求</div>
          <div className="num">{formatNumber(data.today.requests)}</div>
          <div className="metric-change">
            成功 {formatNumber(data.today.success_requests)} · 拒绝 {formatNumber(data.today.rejected_requests)}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">今日花费</div>
          <div className="num">{formatCost(data.today.cost)}</div>
          <div className="metric-change">币种 {data.currency}</div>
        </div>
        <div className="card stat">
          <div className="sub">今日错误率</div>
          <div className="num">{formatErrorRate(errorRate)}</div>
          <div className="metric-change">
            错误 {formatNumber(data.today.error_requests)} · 断开 {formatNumber(data.today.client_disconnect_requests)}
          </div>
        </div>
        <div className="card stat">
          <div className="sub">最近 {data.days} 天请求</div>
          <div className="num">{formatNumber(data.totals.requests)}</div>
          <div className="metric-change">花费 {formatCost(data.totals.cost)}</div>
        </div>
      </div>

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>请求趋势</h3>
            <span className="sub">最近 {data.days} 天</span>
          </div>
          {data.trend.length === 0 ? (
            <div className="chart-empty">暂无趋势数据</div>
          ) : (
            <TrendChart trend={data.trend} />
          )}
        </div>
        <div className="card">
          <div className="row between">
            <h3>今日状态</h3>
            <span className="sub">按请求结果</span>
          </div>
          {statusEntries.length === 0 ? (
            <div className="chart-empty">今日暂无请求</div>
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
            <h3>Token 与成本</h3>
            <span className="sub">今日</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <div className="status-row">
              <span className="sub">输入 tokens</span>
              <span style={{ marginLeft: 'auto' }}>{formatNumber(data.today.input_tokens)}</span>
            </div>
            <div className="status-row">
              <span className="sub">输出 tokens</span>
              <span style={{ marginLeft: 'auto' }}>{formatNumber(data.today.output_tokens)}</span>
            </div>
            <div className="status-row">
              <span className="sub">今日成本</span>
              <span style={{ marginLeft: 'auto' }}>{formatCost(data.today.cost)}</span>
            </div>
          </div>
        </div>
        <div className="card">
          <div className="row between">
            <h3>合计</h3>
            <span className="sub">最近 {data.days} 天</span>
          </div>
          <div style={{ marginTop: 10 }}>
            <div className="status-row">
              <span className="sub">成功 / 错误</span>
              <span style={{ marginLeft: 'auto' }}>
                {formatNumber(data.totals.success_requests)} / {formatNumber(data.totals.error_requests)}
              </span>
            </div>
            <div className="status-row">
              <span className="sub">输入 / 输出 tokens</span>
              <span style={{ marginLeft: 'auto' }}>
                {formatNumber(data.totals.input_tokens)} / {formatNumber(data.totals.output_tokens)}
              </span>
            </div>
            <div className="status-row">
              <span className="sub">总成本</span>
              <span style={{ marginLeft: 'auto' }}>{formatCost(data.totals.cost)}</span>
            </div>
          </div>
        </div>
      </div>
    </>
  )
}
