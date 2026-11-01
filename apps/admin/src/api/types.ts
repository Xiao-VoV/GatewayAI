export interface AdminUser {
  id: string
  email: string
  display_name: string
  status: 'active' | 'disabled'
  created_at: string
}

export interface SetupStatus {
  required: boolean
  setup_token_required: boolean
}

export interface LoginResponse {
  user: AdminUser
  csrf_token: string
  expires_at: string
}

export interface Problem {
  type?: string
  title?: string
  status?: number
  code?: string
  detail?: string
  errors?: Record<string, string>
}

/** metering `/api/admin/overview` metrics (flattened into today/trend/totals). */
export interface Metrics {
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

export interface TodayMetrics extends Metrics {
  day: string
  status_breakdown: Record<string, number>
}

export interface TrendDay extends Metrics {
  day: string
}

export interface Overview {
  currency: 'CNY'
  timezone: 'Asia/Shanghai'
  generated_at: string
  days: number
  today: TodayMetrics
  trend: TrendDay[]
  totals: Metrics
}
