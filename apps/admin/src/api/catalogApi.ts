import { request } from './client'
import type {
  DeploymentAdmin,
  DeploymentCreateInput,
  DeploymentPatchInput,
  ModelGroupAdmin,
  ModelGroupCreateInput,
  ModelGroupPatchInput,
  ModelGroupSummary,
  Page,
  RoutePreview,
} from './catalog'

export interface ModelGroupFilters {
  q?: string
  status?: string
  canonical?: string
  limit?: number
  offset?: number
}

function queryString(filters: ModelGroupFilters): string {
  const params = new URLSearchParams()
  if (filters.q) params.set('q', filters.q)
  if (filters.status) params.set('status', filters.status)
  if (filters.canonical) params.set('canonical', filters.canonical)
  if (filters.limit !== undefined) params.set('limit', String(filters.limit))
  if (filters.offset !== undefined) params.set('offset', String(filters.offset))
  const value = params.toString()
  return value ? `?${value}` : ''
}

export function listModelGroups(filters: ModelGroupFilters = {}) {
  return request<Page<ModelGroupSummary>>(`/api/admin/model-groups${queryString(filters)}`)
}

export function createModelGroup(input: ModelGroupCreateInput) {
  return request<ModelGroupAdmin>('/api/admin/model-groups', { method: 'POST', body: input })
}

export function getModelGroup(groupId: string) {
  return request<ModelGroupAdmin>(`/api/admin/model-groups/${encodeURIComponent(groupId)}`)
}

export function patchModelGroup(groupId: string, patch: ModelGroupPatchInput) {
  return request<ModelGroupAdmin>(`/api/admin/model-groups/${encodeURIComponent(groupId)}`, {
    method: 'PATCH',
    body: patch,
  })
}

export function archiveModelGroup(groupId: string) {
  return request<void>(`/api/admin/model-groups/${encodeURIComponent(groupId)}`, { method: 'DELETE' })
}

export function previewModelGroupRoutes(groupId: string) {
  return request<RoutePreview>(`/api/admin/model-groups/${encodeURIComponent(groupId)}/route-preview`)
}

export function createDeployment(input: DeploymentCreateInput) {
  return request<DeploymentAdmin>('/api/admin/deployments', { method: 'POST', body: input })
}

export function patchDeployment(deploymentId: string, patch: DeploymentPatchInput) {
  return request<DeploymentAdmin>(`/api/admin/deployments/${encodeURIComponent(deploymentId)}`, {
    method: 'PATCH',
    body: patch,
  })
}

export function deleteDeployment(deploymentId: string) {
  return request<void>(`/api/admin/deployments/${encodeURIComponent(deploymentId)}`, { method: 'DELETE' })
}
