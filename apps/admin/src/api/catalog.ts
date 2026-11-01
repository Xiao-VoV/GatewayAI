export interface Page<T> {
  items: T[]
  total: number
  limit: number
  offset: number
}

export interface PublicationSummary {
  id: string
  status: string
  target_count: number
  updated_at: string
}

export interface ModelGroupSummary {
  id: string
  client_model_name: string
  display_name: string
  canonical: string
  status: 'draft' | 'active' | 'archived'
  capabilities: string[]
  context_window: number | null
  max_output: number | null
  deployment_count: number
  enabled_deployment_count: number
  publication_status: string | null
  tool_default_count: number
  warnings: string[]
  created_at: string
  updated_at: string
}

export interface DeploymentAdmin {
  id: string
  model_group_id: string | null
  model_group_client_name: string | null
  upstream_model_id: string
  upstream_model: string
  canonical: string
  provider_id: string
  provider_name: string
  provider_kind: string
  provider_status: string
  provider_account_id: string
  account_name: string
  account_region: string
  account_status: string
  account_health: string
  priority: number
  weight: number
  enabled: boolean
  price_input: number | null
  price_output: number | null
  currency: string
  context_window: number | null
  max_output: number | null
  notes: string
  warnings: string[]
  created_at: string
  updated_at: string
}

export interface ModelGroupAdmin extends ModelGroupSummary {
  description: string
  protocols: string[]
  parameters: unknown
  price_input: number | null
  price_output: number | null
  currency: string
  deployments: DeploymentAdmin[]
  publications: PublicationSummary[]
}

export interface RoutePreviewCandidate {
  deployment_id: string
  upstream_model_id: string
  upstream_model: string
  provider_id: string
  provider_name: string
  provider_status: string
  provider_account_id: string
  account_name: string
  account_status: string
  account_health: string
  priority: number
  weight: number
  enabled: boolean
  warnings: string[]
}

export interface RoutePreview {
  model_group_id: string
  client_model_name: string
  status: string
  candidates: RoutePreviewCandidate[]
  warnings: string[]
}

export interface ModelGroupCreateInput {
  client_model_name: string
  display_name?: string
  canonical: string
  description?: string
  capabilities?: string[]
  context_window?: number | null
  max_output?: number | null
  protocols?: string[]
  status?: string
  deployments?: { upstream_model_id: string; provider_account_id: string; priority?: number; weight?: number }[]
}

export interface ModelGroupPatchInput {
  client_model_name?: string
  display_name?: string
  canonical?: string
  description?: string
  capabilities?: string[]
  context_window?: number | null
  max_output?: number | null
  protocols?: string[]
  status?: string
}

export interface DeploymentCreateInput {
  upstream_model_id: string
  provider_account_id: string
  model_group_id?: string | null
  priority?: number
  weight?: number
  enabled?: boolean
  notes?: string
}

export interface DeploymentPatchInput {
  model_group_id?: string | null
  priority?: number
  weight?: number
  enabled?: boolean
  notes?: string
}
