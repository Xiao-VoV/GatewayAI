import { request } from './client'
import type {
  AccountList,
  CreateAccountInput,
  CreateModelInput,
  CreateProviderInput,
  ModelList,
  Provider,
  ProviderAccount,
  ProviderDetail,
  ProviderPage,
  TestResult,
  UpstreamModel,
} from './providers'

export interface ProviderFilters {
  status?: string
  kind?: string
  limit?: number
  offset?: number
}

function queryString(filters: ProviderFilters): string {
  const params = new URLSearchParams()
  if (filters.status) params.set('status', filters.status)
  if (filters.kind) params.set('kind', filters.kind)
  if (filters.limit !== undefined) params.set('limit', String(filters.limit))
  if (filters.offset !== undefined) params.set('offset', String(filters.offset))
  const value = params.toString()
  return value ? `?${value}` : ''
}

export function listProviders(filters: ProviderFilters = {}) {
  return request<ProviderPage>(`/api/admin/providers${queryString(filters)}`)
}

export function createProvider(input: CreateProviderInput) {
  return request<{ provider: Provider; account: ProviderAccount }>('/api/admin/providers', {
    method: 'POST',
    body: input,
  })
}

export function getProvider(providerId: string) {
  return request<ProviderDetail>(`/api/admin/providers/${encodeURIComponent(providerId)}`)
}

export function patchProvider(
  providerId: string,
  patch: Partial<Pick<Provider, 'name' | 'base_url' | 'description' | 'status'>>,
) {
  return request<Provider>(`/api/admin/providers/${encodeURIComponent(providerId)}`, {
    method: 'PATCH',
    body: patch,
  })
}

export function deleteProvider(providerId: string) {
  return request<void>(`/api/admin/providers/${encodeURIComponent(providerId)}`, { method: 'DELETE' })
}

export function testProvider(providerId: string) {
  return request<TestResult>(`/api/admin/providers/${encodeURIComponent(providerId)}/test`, {
    method: 'POST',
    body: {},
  })
}

export function listAccounts(providerId: string) {
  return request<AccountList>(`/api/admin/providers/${encodeURIComponent(providerId)}/accounts`)
}

export function createAccount(providerId: string, input: CreateAccountInput) {
  return request<ProviderAccount>(`/api/admin/providers/${encodeURIComponent(providerId)}/accounts`, {
    method: 'POST',
    body: input,
  })
}

export function patchAccount(
  providerId: string,
  accountId: string,
  patch: {
    name?: string
    api_key?: string
    region?: string
    weight?: number
    priority?: number
    rpm_limit?: number | null
    tpm_limit?: number | null
    status?: 'active' | 'disabled'
  },
) {
  return request<ProviderAccount>(
    `/api/admin/providers/${encodeURIComponent(providerId)}/accounts/${encodeURIComponent(accountId)}`,
    { method: 'PATCH', body: patch },
  )
}

export function deleteAccount(providerId: string, accountId: string) {
  return request<void>(
    `/api/admin/providers/${encodeURIComponent(providerId)}/accounts/${encodeURIComponent(accountId)}`,
    { method: 'DELETE' },
  )
}

export function testAccount(providerId: string, accountId: string) {
  return request<TestResult>(
    `/api/admin/providers/${encodeURIComponent(providerId)}/accounts/${encodeURIComponent(accountId)}/test`,
    { method: 'POST' },
  )
}

export function listModels(providerId: string) {
  return request<ModelList>(`/api/admin/providers/${encodeURIComponent(providerId)}/models`)
}

export function createModel(providerId: string, input: CreateModelInput) {
  return request<UpstreamModel>(`/api/admin/providers/${encodeURIComponent(providerId)}/models`, {
    method: 'POST',
    body: input,
  })
}

export function deleteModel(providerId: string, modelId: string) {
  return request<void>(
    `/api/admin/providers/${encodeURIComponent(providerId)}/models/${encodeURIComponent(modelId)}`,
    { method: 'DELETE' },
  )
}
