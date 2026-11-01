import { isDesktopRuntime } from './secureStore'

export interface AgentInfo {
  id: string
  name: string
  installed: boolean
  status: 'configured' | 'available' | 'missing'
  command: string | null
  configPath: string | null
  configExists: boolean
  configFormat: string
  supportsWrite: boolean
  endpoint: string | null
  notes: string[]
}

export interface AgentScan {
  os: string
  agents: AgentInfo[]
  warnings: string[]
}

export interface ConfigChange {
  key: string
  value: string
}

export interface AgentPlan {
  agentId: string
  agentName: string
  supported: boolean
  targetPath: string | null
  format: string
  changes: ConfigChange[]
  warnings: string[]
}

export interface AgentApplyResult {
  agentId: string
  configPath: string
  backupPath: string | null
  applied: ConfigChange[]
}

export interface AgentConfigStatus {
  applied: boolean
  configPath: string
  manifestPath: string
  backupPath: string | null
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const api = await import('@tauri-apps/api/core')
  return api.invoke<T>(command, args)
}

function ensureDesktop() {
  if (!isDesktopRuntime()) throw new Error('AI 工具管理需要 Tauri 桌面版')
}

export async function scanAgents(): Promise<AgentScan> {
  ensureDesktop()
  return invoke<AgentScan>('scan_agents')
}

export async function previewAgentConfig(
  agentId: string,
  localBaseUrl: string,
  model?: string,
): Promise<AgentPlan> {
  ensureDesktop()
  return invoke<AgentPlan>('preview_agent_config', {
    agentId,
    localBaseUrl,
    model: model ?? null,
  })
}

export async function applyAgentConfig(
  agentId: string,
  localBaseUrl: string,
  model?: string,
): Promise<AgentApplyResult> {
  ensureDesktop()
  return invoke<AgentApplyResult>('apply_agent_config', {
    agentId,
    localBaseUrl,
    model: model ?? null,
  })
}

export async function rollbackAgentConfig(agentId: string): Promise<void> {
  ensureDesktop()
  await invoke('rollback_agent_config', { agentId })
}

export async function agentConfigStatus(agentId: string): Promise<AgentConfigStatus> {
  ensureDesktop()
  return invoke<AgentConfigStatus>('agent_config_status', { agentId })
}
