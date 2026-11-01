import { request } from './client'
import type { Publication, PublicationCreateInput, PublicationPage, PublicationUpdateInput } from './publications'

export function listPublications(modelGroupId?: string) {
  const params = new URLSearchParams()
  if (modelGroupId) params.set('model_group_id', modelGroupId)
  params.set('limit', '100')
  return request<PublicationPage>(`/api/admin/publications?${params.toString()}`)
}

export function createPublication(input: PublicationCreateInput) {
  return request<Publication>('/api/admin/publications', { method: 'POST', body: input })
}

export function patchPublication(publicationId: string, patch: PublicationUpdateInput) {
  return request<Publication>(`/api/admin/publications/${encodeURIComponent(publicationId)}`, {
    method: 'PATCH',
    body: patch,
  })
}

export function deletePublication(publicationId: string) {
  return request<void>(`/api/admin/publications/${encodeURIComponent(publicationId)}`, { method: 'DELETE' })
}
