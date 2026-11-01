import { request } from './client'
import type { Page } from './catalog'

export interface ModelGroupRef {
  id: string
  client_model_name: string
}

export interface ToolModelDefault {
  id: string
  tool_key: string
  scope_type: 'global' | 'user_group' | 'user'
  scope_id: string | null
  scope_label: string | null
  default_model_group_id: string
  default_model_client_name: string
  allow_switch: boolean
  allowed_model_groups: ModelGroupRef[]
  created_at: string
  updated_at: string
}

export interface ToolDefaultCreateInput {
  tool_key: string
  scope_type: 'global' | 'user_group' | 'user'
  scope_id?: string | null
  default_model_group_id: string
  allow_switch?: boolean
  allowed_model_group_ids?: string[]
}

export interface ToolDefaultUpdateInput {
  default_model_group_id?: string
  allow_switch?: boolean
  allowed_model_group_ids?: string[]
}

export interface ToolDefaultFilters {
  tool_key?: string
  scope_type?: string
  scope_id?: string
  limit?: number
  offset?: number
}

export type ToolDefaultPage = Page<ToolModelDefault>

export function listToolDefaults(filters: ToolDefaultFilters = {}) {
  const params = new URLSearchParams()
  if (filters.tool_key) params.set('tool_key', filters.tool_key)
  if (filters.scope_type) params.set('scope_type', filters.scope_type)
  if (filters.scope_id) params.set('scope_id', filters.scope_id)
  params.set('limit', String(filters.limit ?? 100))
  if (filters.offset !== undefined) params.set('offset', String(filters.offset))
  return request<ToolDefaultPage>(`/api/admin/tool-model-defaults?${params.toString()}`)
}

export function createToolDefault(input: ToolDefaultCreateInput) {
  return request<ToolModelDefault>('/api/admin/tool-model-defaults', { method: 'POST', body: input })
}

export function updateToolDefault(id: string, input: ToolDefaultUpdateInput) {
  return request<ToolModelDefault>(`/api/admin/tool-model-defaults/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: input,
  })
}

export function deleteToolDefault(id: string) {
  return request<void>(`/api/admin/tool-model-defaults/${encodeURIComponent(id)}`, { method: 'DELETE' })
}
