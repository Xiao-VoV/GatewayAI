import { request } from './client'
import type { Overview } from './types'

export function getOverview(days = 7) {
  return request<Overview>(`/api/admin/overview?days=${encodeURIComponent(days)}`)
}
