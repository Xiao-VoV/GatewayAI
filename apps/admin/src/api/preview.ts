import { request } from './client'

export interface PreviewModel {
  id: string
  object: string
  created: number
  owned_by: string
  display_name: string
  upstream_model: string | null
  capabilities: string[]
  context_window: number | null
  max_output: number | null
  protocols: string[]
  visible: boolean
  allowed: boolean
  matched_by: string | null
  publication_id: string | null
}

export interface PreviewToolDefault {
  tool_key: string
  scope_type: string
  source: string
  default_model: string | null
  switchable_models: string[]
  warnings: string[]
}

export interface CatalogPreview {
  catalog_version: string
  subject: {
    user_id: string
    email: string
    display_name: string
    user_status: string
    device_id: string | null
    device_name: string | null
    key_id: string | null
    key_allowed_models: string[]
    user_group_ids: string[]
  }
  models: PreviewModel[]
  tool_defaults: PreviewToolDefault[]
  enforcement: { visible: string; allowed: string; quota: string; data_level: string }
  warnings: string[]
}

export function getCatalogPreview(userId: string, deviceId?: string, keyId?: string) {
  const params = new URLSearchParams({ user_id: userId })
  if (deviceId) params.set('device_id', deviceId)
  if (keyId) params.set('key_id', keyId)
  return request<CatalogPreview>(`/api/admin/catalog/preview?${params.toString()}`)
}
