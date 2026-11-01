import type { Page } from './catalog'

export interface UsageBlock {
  input_tokens: number
  output_tokens: number
  cost: string
  currency: string
  latency_ms: number
  ttft_ms: number | null
  is_stream: boolean
  fallback: boolean
  status: string
  http_status: number | null
  error_code: string | null
  usage_source: string
}

export interface RequestLogItem {
  id: string
  request_id: string
  trace_id: string | null
  created_at: string
  method: string
  path: string
  protocol: string
  upstream_protocol: string | null
  adapter: string | null
  user_id: string | null
  device_id: string | null
  key_id: string | null
  key_prefix: string | null
  model_alias: string | null
  upstream_model: string | null
  provider: string | null
  provider_id: string | null
  is_stream: boolean
  fallback: boolean
  attempt_count: number
  status_code: number | null
  latency_ms: number
  ttft_ms: number | null
  response_size: number | null
  error_code: string | null
  client_ip: string | null
  user_agent: string
}

export interface RequestLogDetail extends RequestLogItem {
  usage: UsageBlock | null
}

export interface RequestLogPage extends Page<RequestLogItem> {}

export interface RequestFilters {
  from?: string
  to?: string
  user_id?: string
  device_id?: string
  key_id?: string
  key_prefix?: string
  model?: string
  provider?: string
  status_class?: string
  status_code?: string
  trace_id?: string
  limit?: number
  offset?: number
}

export interface AuditItem {
  id: string
  created_at: string
  actor_type: string
  actor_id: string | null
  actor_email: string | null
  action: string
  resource_type: string
  resource_id: string | null
  outcome: string
  client_ip: string | null
  user_agent: string
  request_id: string | null
  detail: unknown
}

export interface AuditPage extends Page<AuditItem> {}

export interface AuditFilters {
  from?: string
  to?: string
  actor_id?: string
  action?: string
  resource_type?: string
  resource_id?: string
  outcome?: string
  limit?: number
  offset?: number
}
