import { request } from './client'

export interface Settings {
  default_provider_id: string | null
  request_log_retention_days: number
  audit_retention_days: number
  upstream_timeout_secs: number
  updated_at: string
}

export interface SettingsPatch {
  default_provider_id?: string | null
  request_log_retention_days?: number
  audit_retention_days?: number
  upstream_timeout_secs?: number
}

export function getSettings() {
  return request<Settings>('/api/admin/settings')
}

export function patchSettings(patch: SettingsPatch) {
  return request<Settings>('/api/admin/settings', { method: 'PATCH', body: patch })
}
