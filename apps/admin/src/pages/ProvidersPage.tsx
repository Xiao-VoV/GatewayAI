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
import type { Translate } from '../i18n/translate'

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
    accountName: tr("main_account"),
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
  if (provider.status === 'active') return <Badge tone="ok">{tr("enabled")}</Badge>
  return <Badge>{tr("disabled_2")}</Badge>
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
        setFormError(tr("save_failed_please_try_again"))
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
      const message = err instanceof ApiError ? err.detail : tr("test_request_failed")
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
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr("providers")}
        desc={tr("upstream_services_and_channel_accounts_test_connectivity")}
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr("add_provider")}
            </button>
          </>
        }
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 180 }}>
            <label htmlFor="provider-status">{tr("status")}</label>
            <select
              id="provider-status"
              className="select"
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value)}
            >
              <option value="">{tr("all")}</option>
              <option value="active">{tr("enabled")}</option>
              <option value="disabled">{tr("disabled_2")}</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            {tr("openai_compatible_channels_only_credentials_are_encrypte")}
          </span>
        </div>
      </div>

      {loading && !data ? (
        <LoadingPanel title={tr("loading_providers")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={
            statusFilter
              ? tr("no_matching_providers")
              : tr("no_providers_yet")
          }
          desc={tr("add_an_openai_compatible_provider_and_its_first_account")}
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr("add_provider")}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("name")}</th>
                <th>{tr("type")}</th>
                <th>Base URL</th>
                <th>{tr("description")}</th>
                <th>{tr("status")}</th>
                <th>{tr("last_test")}</th>
                <th className="cell-actions">{tr("actions")}</th>
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
                            {tr("passed")} · {result.latency_ms}ms
                          </Badge>
                        ) : (
                          <Badge tone="bad">{result.error?.code ?? tr("failed")}</Badge>
                        )
                      ) : (
                        <span className="hint">{tr("not_tested")}</span>
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
                          {testingId === provider.id ? tr("testing") : tr("test")}
                        </button>
                        <button
                          className="btn sm"
                          type="button"
                          onClick={(event) => {
                            event.stopPropagation()
                            navigate(`/providers/${provider.id}`)
                          }}
                        >
                          {tr("details")}
                        </button>
                      </span>
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {`${tr("total")} ${data.total} ${tr("providers_2")}`}
          </div>
        </div>
      )}

      {modalOpen && (
        <Modal
          title={tr("add_provider")}
          sub={tr("openai_compatible_channel_the_first_account_is_saved_wit")}
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={saving}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="create-provider-form" disabled={saving}>
                {saving ? tr("saving") : tr("save_provider")}
              </button>
            </>
          }
        >
          <form id="create-provider-form" className="grid" onSubmit={(event) => void handleCreate(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="provider-name">{tr("name")}</label>
                <input
                  id="provider-name"
                  className="input"
                  placeholder={tr("e_g_deepseek")}
                  value={draft.name}
                  onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                />
                {fieldErrors.name && <span className="field-error">{fieldErrors.name}</span>}
              </div>
              <div className="field">
                <label>{tr("type")}</label>
                <input className="input" value={tr("openai_compatible")} disabled />
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
              <label htmlFor="provider-description">{tr("description")}</label>
              <input
                id="provider-description"
                className="input"
                placeholder={tr("optional_e_g_direct_cloud")}
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
              {fieldErrors.description && <span className="field-error">{fieldErrors.description}</span>}
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="account-name">{tr("account_name")}</label>
                <input
                  id="account-name"
                  className="input"
                  value={draft.accountName}
                  onChange={(event) => setDraft({ ...draft, accountName: event.target.value })}
                />
                {fieldErrors['account.name'] && <span className="field-error">{fieldErrors['account.name']}</span>}
              </div>
              <div className="field">
                <label htmlFor="account-region">{tr("region")}</label>
                <input
                  id="account-region"
                  className="input"
                  placeholder={tr("optional_e_g_cn_east_1_us_east_1")}
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
                {tr("stored_encrypted_only_a_hint_is_returned_after_saving_th")}
              </span>
            </div>
            <div className="grid cols-4">
              <div className="field">
                <label htmlFor="account-weight">{tr("weight")}</label>
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
                <label htmlFor="account-priority">{tr("priority")}</label>
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
                <label htmlFor="account-rpm">{tr("rpm_limit")}</label>
                <input
                  id="account-rpm"
                  className="input"
                  type="number"
                  placeholder={tr("optional")}
                  value={draft.rpmLimit}
                  onChange={(event) => setDraft({ ...draft, rpmLimit: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="account-tpm">{tr("tpm_limit")}</label>
                <input
                  id="account-tpm"
                  className="input"
                  type="number"
                  placeholder={tr("optional")}
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
