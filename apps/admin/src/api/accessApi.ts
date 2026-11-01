import { request } from './client'
import type {
  AdminUser,
  CreatePairingCodeResponse,
  DeviceAdmin,
  KeyAdmin,
  Page,
  UserGroupDetail,
  UserGroupSummary,
  ResetKeyResponse,
  PairingCode,
} from './access'

export function listUsers(limit = 200) {
  return request<Page<AdminUser>>(`/api/admin/users?limit=${limit}`)
}

export function listUserGroups(limit = 200) {
  return request<Page<UserGroupSummary>>(`/api/admin/user-groups?limit=${limit}`)
}


export function listDevices(limit = 200, userId?: string) {
  const params = new URLSearchParams({ limit: String(limit) })
  if (userId) params.set('user_id', userId)
  return request<Page<DeviceAdmin>>(`/api/admin/devices?${params.toString()}`)
}

export function listKeys(limit = 200, userId?: string) {
  const params = new URLSearchParams({ limit: String(limit) })
  if (userId) params.set('user_id', userId)
  return request<Page<KeyAdmin>>(`/api/admin/keys?${params.toString()}`)
}

export interface CreateUserGroupInput {
  name: string
  description?: string
  member_ids?: string[]
}

export interface UpdateUserGroupInput {
  name?: string
  description?: string | null
  member_ids?: string[]
}

export function updateUser(userId: string, patch: { display_name?: string; status?: 'active' | 'disabled' }) {
  return request<AdminUser>(`/api/admin/users/${encodeURIComponent(userId)}`, { method: 'PATCH', body: patch })
}

export function getUserGroup(groupId: string) {
  return request<UserGroupDetail>(`/api/admin/user-groups/${encodeURIComponent(groupId)}`)
}

export function createUserGroup(input: CreateUserGroupInput) {
  return request<UserGroupDetail>('/api/admin/user-groups', { method: 'POST', body: input })
}

export function updateUserGroup(groupId: string, patch: UpdateUserGroupInput) {
  return request<UserGroupDetail>(`/api/admin/user-groups/${encodeURIComponent(groupId)}`, {
    method: 'PATCH',
    body: patch,
  })
}

export function deleteUserGroup(groupId: string) {
  return request<void>(`/api/admin/user-groups/${encodeURIComponent(groupId)}`, { method: 'DELETE' })
}

export function createPairingCode(input: { user_email: string; user_display_name?: string; expires_in_seconds?: number }) {
  return request<CreatePairingCodeResponse>('/api/admin/pairing-codes', { method: 'POST', body: input })
}

export interface KeyFilters {
  status?: string
  user_id?: string
  limit?: number
  offset?: number
}

export function listKeysFiltered(filters: KeyFilters = {}) {
  const params = new URLSearchParams()
  if (filters.status) params.set('status', filters.status)
  if (filters.user_id) params.set('user_id', filters.user_id)
  params.set('limit', String(filters.limit ?? 200))
  if (filters.offset !== undefined) params.set('offset', String(filters.offset))
  return request<Page<KeyAdmin>>(`/api/admin/keys?${params.toString()}`)
}

export function updateKey(
  keyId: string,
  patch: { allowed_models?: string[]; rpm_limit?: number | null; expires_at?: string | null },
) {
  return request<KeyAdmin>(`/api/admin/keys/${encodeURIComponent(keyId)}`, { method: 'PATCH', body: patch })
}

export function resetKey(keyId: string) {
  return request<ResetKeyResponse>(`/api/admin/keys/${encodeURIComponent(keyId)}/reset`, { method: 'POST' })
}

export function revokeKey(keyId: string) {
  return request<void>(`/api/admin/keys/${encodeURIComponent(keyId)}`, { method: 'DELETE' })
}

export function listDevicesFiltered(filters: { status?: string; user_id?: string; limit?: number } = {}) {
  const params = new URLSearchParams()
  if (filters.status) params.set('status', filters.status)
  if (filters.user_id) params.set('user_id', filters.user_id)
  params.set('limit', String(filters.limit ?? 200))
  return request<Page<DeviceAdmin>>(`/api/admin/devices?${params.toString()}`)
}

export function updateDevice(deviceId: string, patch: { name?: string; status?: 'active' | 'disabled' }) {
  return request<DeviceAdmin>(`/api/admin/devices/${encodeURIComponent(deviceId)}`, {
    method: 'PATCH',
    body: patch,
  })
}

export function unbindDevice(deviceId: string) {
  return request<void>(`/api/admin/devices/${encodeURIComponent(deviceId)}`, { method: 'DELETE' })
}

export function listPairingCodes(limit = 50) {
  return request<Page<PairingCode>>(`/api/admin/pairing-codes?limit=${limit}`)
}

export function invalidatePairingCode(id: string) {
  return request<void>(`/api/admin/pairing-codes/${encodeURIComponent(id)}`, { method: 'DELETE' })
}
