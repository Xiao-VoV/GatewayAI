import type { Page } from './catalog'

export interface PublicationTarget {
  id: string
  subject_type: 'all_users' | 'user' | 'user_group' | 'application' | 'device'
  subject_id: string | null
  subject_label: string | null
  visible: boolean
  allowed: boolean
}

export interface Publication {
  id: string
  model_group_id: string
  model_group_client_name: string
  status: 'active' | 'paused'
  data_level_max: number | null
  quota_rpm: number | null
  quota_tpm: number | null
  quota_daily_requests: number | null
  note: string
  created_by: string | null
  targets: PublicationTarget[]
  enforcement: {
    visible: string
    allowed: string
    quota: string
    data_level: string
  }
  created_at: string
  updated_at: string
}

export interface PublicationTargetInput {
  subject_type: 'all_users' | 'user' | 'user_group'
  subject_id?: string | null
  visible: boolean
  allowed: boolean
}

export interface PublicationCreateInput {
  model_group_id: string
  status?: 'active' | 'paused'
  data_level_max?: number | null
  quota_rpm?: number | null
  quota_tpm?: number | null
  quota_daily_requests?: number | null
  note?: string
  targets: PublicationTargetInput[]
}

export interface PublicationUpdateInput {
  status?: 'active' | 'paused'
  data_level_max?: number | null
  quota_rpm?: number | null
  quota_tpm?: number | null
  quota_daily_requests?: number | null
  note?: string
  targets?: PublicationTargetInput[]
}

export type PublicationPage = Page<Publication>
