export interface ClientUser {
  id: string
  email: string
  display_name: string
  status: string
}

export interface ClientDevice {
  id: string
  name: string
  platform: string
  client_version: string
  status: string
  last_seen_at: string | null
}

export interface ClientDeviceSummary {
  id: string
  name: string
  platform: string
  status: string
  created_at: string
}

export interface ClientKey {
  id: string
  key_prefix: string
  status: string
  expires_at: string | null
  allowed_models: string[]
}

export interface PairResponse {
  device: ClientDeviceSummary
  user: ClientUser
  device_secret: string
  api_key: string
  api_key_prefix: string
  token: string
  token_type: string
  token_expires_at: string
}

export interface TokenResponse {
  token: string
  token_type: string
  expires_at: string
  device_id: string
}

export interface ClientDeviceResponse {
  device: ClientDevice
  user: ClientUser
  key: ClientKey
}

export interface ClientModel {
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
}

export interface ClientModelList {
  object: string
  data: ClientModel[]
  catalog_version?: string
}

export interface ClientToolDefault {
  tool_key: string
  default_model: string
  allow_switch: boolean
  switchable_models: string[]
}

export interface ClientToolDefaults {
  object: string
  data: ClientToolDefault[]
  catalog_version: string
}

export interface UsageMetrics {
  requests: number
  success_requests: number
  error_requests: number
  rejected_requests: number
  client_disconnect_requests: number
  input_tokens: number
  output_tokens: number
  cost: string
  error_rate?: number
}

export interface UsageToday extends UsageMetrics {
  day: string
  status_breakdown: Record<string, number>
}

export interface UsageTrendDay extends UsageMetrics {
  day: string
}

export interface ClientUsage {
  currency: string
  timezone: string
  generated_at: string
  days: number
  today: UsageToday
  trend: UsageTrendDay[]
  totals: UsageMetrics
}
