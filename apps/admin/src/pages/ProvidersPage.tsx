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
import { useI18n } from '../i18n/I18nProvider'

type Translate = (zh: string, en: string) => string

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

function makeEmptyDraft(tr: Translate): ProviderDraft {
  return {
    name: '',
    base_url: '',
    description: '',
    accountName: tr('主账号', 'Main account'),
    apiKey: '',
    region: '',
    weight: 100,
    priority: 1,
    rpmLimit: '',
    tpmLimit: '',
  }
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
  const { tr } = useI18n()
  if (provider.status === 'active') return <Badge tone="ok">{tr('启用', 'Enabled')}</Badge>
  return <Badge>{tr('已禁用', 'Disabled')}</Badge>
}

export function ProvidersPage() {
  const navigate = useNavigate()
  const { tr } = useI18n()
  const [statusFilter, setStatusFilter] = useState('')
  const [modalOpen, setModalOpen] = useState(false)
  const [draft, setDraft] = useState<ProviderDraft>(() => makeEmptyDraft(tr))
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
    setDraft(makeEmptyDraft(tr))
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
        setFormError(tr('保存失败，请重试', 'Save failed. Please try again.'))
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
      const message = err instanceof ApiError ? err.detail : tr('测试请求失败', 'Test request failed')
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
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr('供应商', 'Providers')}
        desc={tr(
          '上游服务与渠道账号；保存前测试连通性。',
          'Upstream services and channel accounts. Test connectivity before saving.',
        )}
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr('添加供应商', 'Add provider')}
            </button>
          </>
        }
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 180 }}>
            <label htmlFor="provider-status">{tr('状态', 'Status')}</label>
            <select
              id="provider-status"
              className="select"
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value)}
            >
              <option value="">{tr('全部', 'All')}</option>
              <option value="active">{tr('启用', 'Enabled')}</option>
              <option value="disabled">{tr('已禁用', 'Disabled')}</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            {tr(
              '仅支持 OpenAI 兼容渠道；凭证加密存储，仅显示后四位。',
              'OpenAI-compatible channels only. Credentials are encrypted; only the last four characters are shown.',
            )}
          </span>
        </div>
      </div>

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取供应商…', 'Loading providers…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={
            statusFilter
              ? tr('没有匹配的供应商', 'No matching providers')
              : tr('还没有供应商', 'No providers yet')
          }
          desc={tr(
            '添加 OpenAI 兼容供应商与第一个渠道账号；之后可以测试连通性、添加上游模型。',
            'Add an OpenAI-compatible provider and its first account; then test connectivity and add upstream models.',
          )}
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr('添加供应商', 'Add provider')}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('名称', 'Name')}</th>
                <th>{tr('类型', 'Type')}</th>
                <th>Base URL</th>
                <th>{tr('描述', 'Description')}</th>
                <th>{tr('状态', 'Status')}</th>
                <th>{tr('最近测试', 'Last test')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                          <Badge tone="ok">
                            {tr('成功', 'Passed')} · {result.latency_ms}ms
                          </Badge>
                        ) : (
                          <Badge tone="bad">{result.error?.code ?? tr('失败', 'Failed')}</Badge>
                        )
                      ) : (
                        <span className="hint">{tr('未测试', 'Not tested')}</span>
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
                          {testingId === provider.id ? tr('测试中…', 'Testing…') : tr('测试', 'Test')}
                        </button>
                        <button
                          className="btn sm"
                          type="button"
                          onClick={(event) => {
                            event.stopPropagation()
                            navigate(`/providers/${provider.id}`)
                          }}
                        >
                          {tr('详情', 'Details')}
                        </button>
                      </span>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {`${tr('共', 'Total:')} ${data.total} ${tr('个供应商', 'providers')}`}
          </div>
        </div>
      )}

      {modalOpen && (
        <Modal
          title={tr('添加供应商', 'Add provider')}
          sub={tr(
            'OpenAI 兼容渠道；创建时同时保存第一个账号',
            'OpenAI-compatible channel; the first account is saved with it',
          )}
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={saving}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="create-provider-form" disabled={saving}>
                {saving ? tr('保存中…', 'Saving…') : tr('保存供应商', 'Save provider')}
              </button>
            </>
          }
        >
          <form id="create-provider-form" className="grid" onSubmit={(event) => void handleCreate(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="provider-name">{tr('名称', 'Name')}</label>
                <input
                  id="provider-name"
                  className="input"
                  placeholder={tr('例如 DeepSeek', 'e.g. DeepSeek')}
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
                {fieldErrors.name && <span className="field-error">{fieldErrors.name}</span>}
              </div>
              <div className="field">
                <label>{tr('类型', 'Type')}</label>
                <input className="input" value={tr('OpenAI 兼容', 'OpenAI compatible')} disabled />
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
              <label htmlFor="provider-description">{tr('描述', 'Description')}</label>
              <input
                id="provider-description"
                className="input"
                placeholder={tr('可选，例如 官方直连 / 云服务', 'Optional, e.g. direct / cloud')}
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
              {fieldErrors.description && <span className="field-error">{fieldErrors.description}</span>}
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="account-name">{tr('账号名', 'Account name')}</label>
                <input
                  id="account-name"
                  className="input"
                  value={draft.accountName}
                  onChange={(event) => setDraft({ ...draft, accountName: event.target.value })}
                />
                {fieldErrors['account.name'] && <span className="field-error">{fieldErrors['account.name']}</span>}
              </div>
              <div className="field">
                <label htmlFor="account-region">{tr('区域', 'Region')}</label>
                <input
                  id="account-region"
                  className="input"
                  placeholder={tr('可选，例如 华东-上海 / us-east-1', 'Optional, e.g. cn-east-1 / us-east-1')}
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
              <span className="hint">
                {tr(
                  '加密存储；保存后只显示提示，不返回明文。',
                  'Stored encrypted. Only a hint is returned after saving; the plaintext key is never exposed.',
                )}
              </span>
            </div>
            <div className="grid cols-4">
              <div className="field">
                <label htmlFor="account-weight">{tr('权重 %', 'Weight %')}</label>
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
                <label htmlFor="account-priority">{tr('优先级', 'Priority')}</label>
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
                <label htmlFor="account-rpm">{tr('RPM 上限', 'RPM limit')}</label>
                <input
                  id="account-rpm"
                  className="input"
                  type="number"
                  placeholder={tr('可选', 'Optional')}
                  value={draft.rpmLimit}
                  onChange={(event) => setDraft({ ...draft, rpmLimit: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="account-tpm">{tr('TPM 上限', 'TPM limit')}</label>
                <input
                  id="account-tpm"
                  className="input"
                  type="number"
                  placeholder={tr('可选', 'Optional')}
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
