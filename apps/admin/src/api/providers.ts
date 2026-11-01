export interface Provider {
  id: string
  name: string
  kind: string
  base_url: string
  description: string
  status: 'active' | 'disabled'
  created_at: string
  updated_at: string
}

export interface ProviderPage {
  items: Provider[]
  total: number
  limit: number
  offset: number
}

export interface ProviderDetail extends Provider {
  account_count: number
  active_account_count: number
  upstream_model_count: number
  model_alias_count: number
  routable: boolean
}

export interface ProviderAccount {
  id: string
  provider_id: string
  name: string
  region: string
  api_key_hint: string
  weight: number
  priority: number
  rpm_limit: number | null
  tpm_limit: number | null
  health: string
  status: 'active' | 'disabled'
  last_checked_at: string | null
  last_error: string | null
  created_at: string
  updated_at: string
}

export interface AccountList {
  items: ProviderAccount[]
}

export interface UpstreamModel {
  id: string
  provider_id: string
  name: string
  canonical: string
  capabilities: string[]
  context_window: number | null
  max_output: number | null
  price_input: number | null
  price_output: number | null
  currency: string
  created_at: string
  updated_at: string
}

export interface ModelList {
  items: UpstreamModel[]
}

export interface TestError {
  code: string
  message: string
}

export interface TestResult {
  ok: boolean
  latency_ms: number
  checked_at: string
  account_id: string | null
  http_status: number | null
  error: TestError | null
}

export interface CreateProviderInput {
  name: string
  kind: 'openai_compatible'
  base_url: string
  description?: string
  account: {
    name: string
    api_key: string
    region?: string
    weight?: number
    priority?: number
    rpm_limit?: number | null
    tpm_limit?: number | null
  }
}

export interface CreateAccountInput {
  name: string
  api_key: string
  region?: string
  weight?: number
  priority?: number
  rpm_limit?: number | null
  tpm_limit?: number | null
}

export interface CreateModelInput {
  name: string
  canonical: string
  capabilities?: string[]
  context_window?: number | null
  max_output?: number | null
  price_input?: number | null
  price_output?: number | null
  currency?: string
}
