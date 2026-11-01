import { apiPath, request } from './client'
import type {
  AuditFilters,
  AuditPage,
  RequestFilters,
  RequestLogDetail,
  RequestLogPage,
} from './observability'

function toQuery(filters: object): string {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(filters) as [string, string | number | null | undefined][]) {
    if (value !== undefined && value !== null && String(value).trim() !== '') params.set(key, String(value))
  }
  const query = params.toString()
  return query ? `?${query}` : ''
}

export function listRequests(filters: RequestFilters = {}) {
  return request<RequestLogPage>(
    `/api/admin/requests${toQuery({ ...filters, limit: filters.limit ?? 100 })}`,
  )
}

export function getRequestDetail(requestId: string) {
  return request<RequestLogDetail>(`/api/admin/requests/${encodeURIComponent(requestId)}`)
}

export function listAudit(filters: AuditFilters = {}) {
  return request<AuditPage>(`/api/admin/audit${toQuery({ ...filters, limit: filters.limit ?? 100 })}`)
}

export function requestExportUrl(filters: RequestFilters = {}) {
  return apiPath(`/api/admin/requests/export${toQuery(filters)}`)
}

export function auditExportUrl(filters: AuditFilters = {}) {
  return apiPath(`/api/admin/audit/export${toQuery(filters)}`)
}

/** 用 fetch 带 Cookie 下载 CSV，失败时把 problem detail 抛给页面。 */
export async function downloadCsv(url: string, fallbackName: string) {
  const response = await fetch(url, { credentials: 'include', headers: { Accept: 'text/csv' } })
  if (!response.ok) {
    let detail = `导出失败（HTTP ${response.status}）`
    try {
      const problem = (await response.json()) as { detail?: string }
      if (problem.detail) detail = problem.detail
    } catch {
      // ignore non-json
    }
    throw new Error(detail)
  }
  const blob = await response.blob()
  const disposition = response.headers.get('content-disposition') ?? ''
  const match = /filename="?([^";]+)"?/.exec(disposition)
  const filename = match?.[1] ?? fallbackName
  const href = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = href
  link.download = filename
  document.body.appendChild(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(href)
}
