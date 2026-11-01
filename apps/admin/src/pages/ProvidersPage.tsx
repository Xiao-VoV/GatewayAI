import { useState } from 'react'
import type { FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ApiError } from '../api/client'
import { createProvider, listProviders, testProvider } from '../api/providersApi'
import type { CreateProviderInput, Provider, TestResult } from '../api/providers'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ErrorPanel, LoadingPanel, EmptyPanel } from '../components/ScreenState'
import { Modal } from '../components/Modal'

interface ProviderDraft {
  name: string
  base_url: string
  description: string
  accountName: string
  apiKey: string
  region: string
  weight: number
  priority: number
  rpmLimit: string
  tpmLimit: string
}

const emptyDraft: ProviderDraft = {
  name: '',
  base_url: '',
  description: '',
  accountName: '主账号',
  apiKey: '',
  region: '',
  weight: 100,
  priority: 1,
  rpmLimit: '',
  tpmLimit: '',
}

function toInput(draft: ProviderDraft): CreateProviderInput {
  return {
    name: draft.name.trim(),
    kind: 'openai_compatible',
    base_url: draft.base_url.trim(),
    description: draft.description.trim(),
    account: {
      name: draft.accountName.trim(),
      api_key: draft.apiKey,
      region: draft.region.trim(),
      weight: draft.weight,
      priority: draft.priority,
      rpm_limit: draft.rpmLimit.trim() ? Number(draft.rpmLimit) : null,
      tpm_limit: draft.tpmLimit.trim() ? Number(draft.tpmLimit) : null,
    },
  }
}

function StatusBadge({ provider }: { provider: Provider }) {
  if (provider.status === 'active') return <Badge tone="ok">启用</Badge>
  return <Badge>已禁用</Badge>
}

export function ProvidersPage() {
  const navigate = useNavigate()
  const [statusFilter, setStatusFilter] = useState('')
  const [modalOpen, setModalOpen] = useState(false)
  const [draft, setDraft] = useState<ProviderDraft>(emptyDraft)
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [testingId, setTestingId] = useState<string | null>(null)
  const [testResults, setTestResults] = useState<Record<string, TestResult>>({})

  const { data, loading, error, reload } = useAsync(
    () => listProviders({ status: statusFilter || undefined, limit: 100 }),
    [statusFilter],
  )

  function openCreate() {
    setDraft(emptyDraft)
    setFieldErrors({})
    setFormError(null)
    setModalOpen(true)
  }

  async function handleCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFormError(null)
    setSaving(true)
    try {
      await createProvider(toInput(draft))
      setModalOpen(false)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setFieldErrors(err.errors)
        setFormError(err.detail)
      } else {
        setFormError('保存失败，请重试')
      }
    } finally {
      setSaving(false)
    }
  }

  async function handleTest(provider: Provider) {
    setTestingId(provider.id)
    try {
      const result = await testProvider(provider.id)
      setTestResults((prev) => ({ ...prev, [provider.id]: result }))
    } catch (err) {
      const message = err instanceof ApiError ? err.detail : '测试请求失败'
      setTestResults((prev) => ({
        ...prev,
        [provider.id]: {
          ok: false,
          latency_ms: 0,
          checked_at: new Date().toISOString(),
          account_id: null,
          http_status: null,
          error: { code: 'request_failed', message },
        },
      }))
    } finally {
      setTestingId(null)
    }
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? '刷新中…' : '刷新'}
    </button>
  )

  return (
    <>
      <PageHead
        title="供应商"
        desc="上游服务与渠道账号；保存前测试连通性。"
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              添加供应商
            </button>
          </>
        }
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 180 }}>
            <label htmlFor="provider-status">状态</label>
            <select
              id="provider-status"
              className="select"
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value)}
            >
              <option value="">全部</option>
              <option value="active">启用</option>
              <option value="disabled">已禁用</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            仅支持 OpenAI 兼容渠道；凭证加密存储，仅显示后四位。
          </span>
        </div>
      </div>

      {loading && !data ? (
        <LoadingPanel title="正在读取供应商…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={statusFilter ? '没有匹配的供应商' : '还没有供应商'}
          desc="添加 OpenAI 兼容供应商与第一个渠道账号；之后可以测试连通性、添加上游模型。"
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              添加供应商
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>名称</th>
                <th>类型</th>
                <th>Base URL</th>
                <th>描述</th>
                <th>状态</th>
                <th>最近测试</th>
                <th className="cell-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {data.items.map((provider) => {
                const result = testResults[provider.id]
                return (
                  <tr key={provider.id} className="clickable" onClick={() => navigate(`/providers/${provider.id}`)}>
                    <td>
                      <strong>{provider.name}</strong>
                      <div className="hint">{provider.id.slice(0, 8)}</div>
                    </td>
                    <td>{provider.kind}</td>
                    <td>
                      <code className="kbd">{provider.base_url}</code>
                    </td>
                    <td>{provider.description || '—'}</td>
                    <td>
                      <StatusBadge provider={provider} />
                    </td>
                    <td>
                      {result ? (
                        result.ok ? (
                          <Badge tone="ok">成功 · {result.latency_ms}ms</Badge>
                        ) : (
                          <Badge tone="bad">{result.error?.code ?? '失败'}</Badge>
                        )
                      ) : (
                        <span className="hint">未测试</span>
                      )}
                    </td>
                    <td className="cell-actions">
                      <span className="row-actions">
                        <button
                          className="btn sm"
                          type="button"
                          disabled={testingId === provider.id}
                          onClick={(event) => {
                            event.stopPropagation()
                            void handleTest(provider)
                          }}
                        >
                          {testingId === provider.id ? '测试中…' : '测试'}
                        </button>
                        <button
                          className="btn sm"
                          type="button"
                          onClick={(event) => {
                            event.stopPropagation()
                            navigate(`/providers/${provider.id}`)
                          }}
                        >
                          详情
                        </button>
                      </span>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            共 {data.total} 个供应商
          </div>
        </div>
      )}

      {modalOpen && (
        <Modal
          title="添加供应商"
          sub="OpenAI 兼容渠道；创建时同时保存第一个账号"
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={saving}>
                取消
              </button>
              <button className="btn primary" type="submit" form="create-provider-form" disabled={saving}>
                {saving ? '保存中…' : '保存供应商'}
              </button>
            </>
          }
        >
          <form id="create-provider-form" className="grid" onSubmit={(event) => void handleCreate(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="provider-name">名称</label>
                <input
                  id="provider-name"
                  className="input"
                  placeholder="例如 DeepSeek"
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
                {fieldErrors.name && <span className="field-error">{fieldErrors.name}</span>}
              </div>
              <div className="field">
                <label>类型</label>
                <input className="input" value="OpenAI 兼容" disabled />
              </div>
            </div>
            <div className="field">
              <label htmlFor="provider-base-url">Base URL</label>
              <input
                id="provider-base-url"
                className="input"
                placeholder="https://api.example.com/v1"
                value={draft.base_url}
                onChange={(event) => setDraft({ ...draft, base_url: event.target.value })}
              />
              {fieldErrors.base_url && <span className="field-error">{fieldErrors.base_url}</span>}
            </div>
            <div className="field">
              <label htmlFor="provider-description">描述</label>
              <input
                id="provider-description"
                className="input"
                placeholder="可选，例如 官方直连 / 云服务"
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
              {fieldErrors.description && <span className="field-error">{fieldErrors.description}</span>}
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="account-name">账号名</label>
                <input
                  id="account-name"
                  className="input"
                  value={draft.accountName}
                  onChange={(event) => setDraft({ ...draft, accountName: event.target.value })}
                />
                {fieldErrors['account.name'] && <span className="field-error">{fieldErrors['account.name']}</span>}
              </div>
              <div className="field">
                <label htmlFor="account-region">区域</label>
                <input
                  id="account-region"
                  className="input"
                  placeholder="可选，例如 华东-上海 / us-east-1"
                  value={draft.region}
                  onChange={(event) => setDraft({ ...draft, region: event.target.value })}
                />
                {fieldErrors['account.region'] && <span className="field-error">{fieldErrors['account.region']}</span>}
              </div>
            </div>
            <div className="field">
              <label htmlFor="account-key">API Key</label>
              <input
                id="account-key"
                className="input"
                type="password"
                autoComplete="off"
                placeholder="sk-..."
                value={draft.apiKey}
                onChange={(event) => setDraft({ ...draft, apiKey: event.target.value })}
              />
              {fieldErrors['account.api_key'] && <span className="field-error">{fieldErrors['account.api_key']}</span>}
              <span className="hint">加密存储；保存后只显示提示，不返回明文。</span>
            </div>
            <div className="grid cols-4">
              <div className="field">
                <label htmlFor="account-weight">权重 %</label>
                <input
                  id="account-weight"
                  className="input"
                  type="number"
                  min={0}
                  max={100}
                  value={draft.weight}
                  onChange={(event) => setDraft({ ...draft, weight: Number(event.target.value) })}
                />
              </div>
              <div className="field">
                <label htmlFor="account-priority">优先级</label>
                <input
                  id="account-priority"
                  className="input"
                  type="number"
                  min={1}
                  max={1000}
                  value={draft.priority}
                  onChange={(event) => setDraft({ ...draft, priority: Number(event.target.value) })}
                />
              </div>
              <div className="field">
                <label htmlFor="account-rpm">RPM 上限</label>
                <input
                  id="account-rpm"
                  className="input"
                  type="number"
                  placeholder="可选"
                  value={draft.rpmLimit}
                  onChange={(event) => setDraft({ ...draft, rpmLimit: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="account-tpm">TPM 上限</label>
                <input
                  id="account-tpm"
                  className="input"
                  type="number"
                  placeholder="可选"
                  value={draft.tpmLimit}
                  onChange={(event) => setDraft({ ...draft, tpmLimit: event.target.value })}
                />
              </div>
            </div>
            {formError && (
              <div className="form-error" role="alert">
                {formError}
              </div>
            )}
          </form>
        </Modal>
      )}
    </>
  )
}
