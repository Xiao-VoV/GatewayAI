import { request } from './client'
import type { AdminUser, LoginResponse, SetupStatus } from './types'

export interface LoginPayload {
  email: string
  password: string
}

export interface SetupPayload {
  email: string
  password: string
  display_name?: string
  setupToken?: string
}

export function getSetupStatus() {
  return request<SetupStatus>('/api/admin/auth/setup')
}

export function login(payload: LoginPayload) {
  return request<LoginResponse>('/api/admin/auth/login', {
    method: 'POST',
    body: payload,
    skipUnauthorizedHandler: true,
  })
}

export function setup(payload: SetupPayload) {
  const headers: HeadersInit = {}
  if (payload.setupToken) headers['X-Setup-Token'] = payload.setupToken
  return request<AdminUser>('/api/admin/auth/setup', {
    method: 'POST',
    headers,
    body: { email: payload.email, password: payload.password, display_name: payload.display_name ?? '' },
    skipUnauthorizedHandler: true,
  })
}

export function me() {
  return request<LoginResponse>('/api/admin/auth/me', { skipUnauthorizedHandler: true })
}

export function logout() {
  return request<void>('/api/admin/auth/logout', { method: 'POST' })
}

export function changePassword(currentPassword: string, newPassword: string) {
  return request<void>('/api/admin/auth/password', {
    method: 'PATCH',
    body: { current_password: currentPassword, new_password: newPassword },
  })
}
