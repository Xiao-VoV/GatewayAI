export interface AdminUser {
  id: string
  email: string
  display_name: string
  status: string
}

export interface UserGroupSummary {
  id: string
  name: string
  description: string
  member_count: number
  created_at: string
  updated_at: string
}

export interface Page<T> {
  items: T[]
  total: number
  limit: number
  offset: number
}

export interface DeviceKeySummary {
  id: string
  key_prefix: string
  status: string
  expires_at: string | null
}

export interface DeviceAdmin {
  id: string
  user: AdminUser
  name: string
  platform: string
  client_version: string
  status: string
  online: boolean
  last_seen_at: string | null
  last_ip: string
  key: DeviceKeySummary | null
  created_at: string
  updated_at: string
}

export interface KeyAdmin {
  id: string
  key_prefix: string
  status: string
  user: AdminUser
  device: { id: string; name: string }
  allowed_models: string[]
  rpm_limit: number | null
  expires_at: string | null
  last_used_at: string | null
  revoked_at: string | null
  created_at: string
  updated_at: string
}

export interface UserGroupDetail {
  id: string
  name: string
  description: string
  members: AdminUser[]
  created_at: string
  updated_at: string
}

export interface PairingCode {
  id: string
  user: AdminUser
  status: string
  expires_at: string
  used_at: string | null
  invalidated_at: string | null
  created_at: string
}

export interface PairingCodeWithCode extends PairingCode {
  code: string
}

export interface CreatePairingCodeResponse {
  pairing_code: PairingCodeWithCode
}

export interface ResetKeyResponse {
  api_key: string
  key: KeyAdmin
}
