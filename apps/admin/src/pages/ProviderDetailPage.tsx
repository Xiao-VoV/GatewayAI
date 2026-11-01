import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { ApiError } from '../api/client'
import {
  createAccount,
  createModel,
  deleteAccount,
  deleteModel,
  deleteProvider,
  getProvider,
  listAccounts,
  listModels,
  patchAccount,
  patchProvider,
  testAccount,
  testProvider,
} from '../api/providersApi'
import type {
  CreateAccountInput,
  CreateModelInput,
  ProviderAccount,
  TestResult,
  UpstreamModel,
} from '../api/providers'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { ConfirmDialog, Modal } from '../components/Modal'
import { useI18n } from '../i18n/I18nProvider'

function fmtPrice(value: number | null) {
  return value === null ? '—' : String(value)
}

function TestBadge({ result }: { result: TestResult | undefined }) {
  const { tr } = useI18n()
  if (!result) return <span className="hint">{tr("not_tested")}</span>
  if (result.ok)
    return (
      <Badge tone="ok">
        {tr("passed")} · {result.latency_ms}ms
      </Badge>
    )
  return <Badge tone="bad">{result.error?.code ?? tr("failed")}</Badge>
}

interface AccountDraft {
  name: string
  api_key: string
  region: string
  weight: number
  priority: number
  rpm_limit: string
  tpm_limit: string
}

interface ModelDraft {
  name: string
  canonical: string
  capabilities: string
  context_window: string
  max_output: string
  price_input: string
  price_output: string
  currency: string
}

const emptyAccount: AccountDraft = {
  name: '',
  api_key: '',
  region: '',
  weight: 100,
  priority: 1,
  rpm_limit: '',
  tpm_limit: '',
}

const emptyModel: ModelDraft = {
  name: '',
  canonical: '',
  capabilities: 'chat, stream',
  context_window: '',
  max_output: '',
  price_input: '',
  price_output: '',
  currency: 'CNY',
}

function accountInput(draft: AccountDraft): CreateAccountInput {
  return {
    name: draft.name.trim(),
    api_key: draft.api_key,
    region: draft.region.trim(),
    weight: draft.weight,
    priority: draft.priority,
    rpm_limit: draft.rpm_limit.trim() ? Number(draft.rpm_limit) : null,
    tpm_limit: draft.tpm_limit.trim() ? Number(draft.tpm_limit) : null,
  }
}

function modelInput(draft: ModelDraft): CreateModelInput {
  return {
    name: draft.name.trim(),
    canonical: draft.canonical.trim(),
    capabilities: draft.capabilities
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean),
    context_window: draft.context_window.trim() ? Number(draft.context_window) : null,
    max_output: draft.max_output.trim() ? Number(draft.max_output) : null,
    price_input: draft.price_input.trim() ? Number(draft.price_input) : null,
    price_output: draft.price_output.trim() ? Number(draft.price_output) : null,
    currency: draft.currency.trim() || 'CNY',
  }
}

export function ProviderDetailPage() {
  const { providerId = '' } = useParams()
  const navigate = useNavigate()
  const { tr, locale } = useI18n()
  const { data, loading, error, reload } = useAsync(
    () =>
      Promise.all([getProvider(providerId), listAccounts(providerId), listModels(providerId)]).then(
        ([provider, accounts, models]) => ({ provider, accounts: accounts.items, models: models.items }),
      ),
    [providerId],
  )

  const [providerTest, setProviderTest] = useState<TestResult | null>(null)
  const [accountTests, setAccountTests] = useState<Record<string, TestResult>>({})
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)

  const [accountModal, setAccountModal] = useState(false)
  const [accountDraft, setAccountDraft] = useState<AccountDraft>(emptyAccount)
  const [accountErrors, setAccountErrors] = useState<Record<string, string>>({})
  const [modelModal, setModelModal] = useState(false)
  const [modelDraft, setModelDraft] = useState<ModelDraft>(emptyModel)
  const [modelErrors, setModelErrors] = useState<Record<string, string>>({})

  const [deleteProviderOpen, setDeleteProviderOpen] = useState(false)
  const [deleteAccountTarget, setDeleteAccountTarget] = useState<ProviderAccount | null>(null)
  const [deleteModelTarget, setDeleteModelTarget] = useState<UpstreamModel | null>(null)

  function handleApiError(err: unknown, fallback: string) {
    setActionError(err instanceof ApiError ? err.detail : fallback)
  }

  async function handleTestProvider() {
    setBusy('provider-test')
    setActionError(null)
    try {
      setProviderTest(await testProvider(providerId))
    } catch (err) {
      handleApiError(err, tr("test_failed"))
    } finally {
      setBusy(null)
    }
  }

  async function handleTestAccount(account: ProviderAccount) {
    setBusy(`account-test-${account.id}`)
    setActionError(null)
    try {
      const result = await testAccount(providerId, account.id)
      setAccountTests((prev) => ({ ...prev, [account.id]: result }))
    } catch (err) {
      handleApiError(err, tr("account_test_failed"))
    } finally {
      setBusy(null)
    }
  }

  async function toggleProviderStatus() {
    if (!data) return
    setBusy('provider-status')
    setActionError(null)
    try {
      await patchProvider(providerId, { status: data.provider.status === 'active' ? 'disabled' : 'active' })
      reload()
    } catch (err) {
      handleApiError(err, tr("failed_to_update_provider_status"))
    } finally {
      setBusy(null)
    }
  }

  async function toggleAccountStatus(account: ProviderAccount) {
    setBusy(`account-status-${account.id}`)
    setActionError(null)
    try {
      await patchAccount(providerId, account.id, { status: account.status === 'active' ? 'disabled' : 'active' })
      reload()
    } catch (err) {
      handleApiError(err, tr("failed_to_update_account_status"))
    } finally {
      setBusy(null)
    }
  }

  async function handleCreateAccount(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('account-create')
    setAccountErrors({})
    setActionError(null)
    try {
      await createAccount(providerId, accountInput(accountDraft))
      setAccountModal(false)
      setAccountDraft(emptyAccount)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setAccountErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError(tr("failed_to_add_account"))
      }
    } finally {
      setBusy(null)
    }
  }

  async function handleCreateModel(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('model-create')
    setModelErrors({})
    setActionError(null)
    try {
      await createModel(providerId, modelInput(modelDraft))
      setModelModal(false)
      setModelDraft(emptyModel)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setModelErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError(tr("failed_to_add_upstream_model"))
      }
    } finally {
      setBusy(null)
    }
  }

  async function handleDeleteProvider() {
    setBusy('provider-delete')
    setActionError(null)
    try {
      await deleteProvider(providerId)
      navigate('/providers', { replace: true })
    } catch (err) {
      handleApiError(err, tr("failed_to_delete_provider"))
      setDeleteProviderOpen(false)
    } finally {
      setBusy(null)
    }
  }

  async function handleDeleteAccount() {
    if (!deleteAccountTarget) return
    setBusy('account-delete')
    setActionError(null)
    try {
      await deleteAccount(providerId, deleteAccountTarget.id)
      setDeleteAccountTarget(null)
      reload()
    } catch (err) {
      handleApiError(err, tr("failed_to_delete_account"))
    } finally {
      setBusy(null)
    }
  }

  async function handleDeleteModel() {
    if (!deleteModelTarget) return
    setBusy('model-delete')
    setActionError(null)
    try {
      await deleteModel(providerId, deleteModelTarget.id)
      setDeleteModelTarget(null)
      reload()
    } catch (err) {
      handleApiError(err, tr("failed_to_delete_upstream_model"))
    } finally {
      setBusy(null)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead
          title={tr("provider_detail")}
          actions={
            <Link className="btn" to="/providers">
              {tr("back_to_list")}
            </Link>
          }
        />
        <LoadingPanel title={tr("loading_providers")} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead
          title={tr("provider_detail")}
          actions={
            <Link className="btn" to="/providers">
              {tr("back_to_list")}
            </Link>
          }
        />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  const { provider, accounts, models } = data

  return (
    <>
      <PageHead
        title={`${tr("provider")} · ${provider.name}`}
        desc={tr("account_pool_upstream_models_and_connectivity")}
        actions={
          <>
            <Link className="btn" to="/providers">
              {tr("back_to_list")}
            </Link>
            <button className="btn" type="button" onClick={() => void handleTestProvider()} disabled={busy !== null}>
              {busy === 'provider-test' ? tr("testing") : tr("test_connectivity")}
            </button>
          </>
        }
      />

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{tr("basics")}</h3>
            <Badge tone={provider.status === 'active' ? 'ok' : ''}>
              {provider.status === 'active' ? tr("enabled") : tr("disabled_2")}
            </Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{tr("type")}</dt>
            <dd>{provider.kind}</dd>
            <dt>Base URL</dt>
            <dd>
              <code className="kbd">{provider.base_url}</code>
            </dd>
            <dt>{tr("description")}</dt>
            <dd>{provider.description || '—'}</dd>
            <dt>{tr("accounts")}</dt>
            <dd>
              {`${provider.account_count} ${tr("accounts_3")} · ${provider.active_account_count} ${tr("active_4")}`}
            </dd>
            <dt>{tr("upstream_models")}</dt>
            <dd>{`${provider.upstream_model_count} ${tr("items")}`}</dd>
            <dt>{tr("referenced_aliases")}</dt>
            <dd>{`${provider.model_alias_count} ${tr("aliases")}`}</dd>
            <dt>{tr("routable")}</dt>
            <dd>
              {provider.routable ? (
                <Badge tone="ok">{tr("yes")}</Badge>
              ) : (
                <Badge tone="warn">{tr("no")}</Badge>
              )}
            </dd>
          </dl>
          <div className="row" style={{ marginTop: 14 }}>
            <button className="btn" type="button" onClick={() => void toggleProviderStatus()} disabled={busy !== null}>
              {provider.status === 'active'
                ? tr("disable_provider")
                : tr("enable_provider")}
            </button>
          </div>
        </div>
        <div className="card">
          <h3>{tr("last_test")}</h3>
          {providerTest ? (
            <div className="statebox" style={{ marginTop: 10 }}>
              <h3>{providerTest.ok ? tr("test_passed") : tr("test_failed")}</h3>
              <p>
                {providerTest.ok
                  ? `${tr("latency")} ${providerTest.latency_ms}ms`
                  : `${providerTest.error?.code ?? tr("failed")} · ${providerTest.error?.message ?? ''}`}
              </p>
              <p className="hint">
                {tr("tested_at")} {new Date(providerTest.checked_at).toLocaleString(locale)}
              </p>
            </div>
          ) : (
            <p className="sub" style={{ marginTop: 10 }}>
              {tr("click_test_connectivity_to_check_network_auth_and_upstre")}
            </p>
          )}
          <dl className="kv" style={{ marginTop: 12 }}>
            <dt>{tr("account_health")}</dt>
            <dd>{tr("updated_by_tests_and_upstream_requests")}</dd>
            <dt>{tr("probe_scope")}</dt>
            <dd>{tr("network_credentials_model_name_429")}</dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <div className="row">
            <h3>{tr("account_pool")}</h3>
            <span className="sub">{`${accounts.length} ${tr("accounts_2")}`}</span>
          </div>
          <button
            className="btn sm primary"
            type="button"
            onClick={() => {
              setAccountDraft(emptyAccount)
              setAccountErrors({})
              setAccountModal(true)
            }}
          >
            {tr("add_account")}
          </button>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("account_2")}</th>
                <th>{tr("region")}</th>
                <th>{tr("weight_2")}</th>
                <th>{tr("priority")}</th>
                <th>RPM</th>
                <th>TPM</th>
                <th>{tr("health")}</th>
                <th>{tr("last_test")}</th>
                <th className="cell-actions">{tr("actions")}</th>
              </tr>
            </thead>
            <tbody>
              {accounts.map((account) => (
                <tr key={account.id} className={account.status === 'disabled' ? 'is-off' : ''}>
                  <td>
                    <strong>{account.name}</strong>
                    <div className="hint">{account.api_key_hint}</div>
                  </td>
                  <td>{account.region || '—'}</td>
                  <td className="num">{account.weight}%</td>
                  <td className="num">{account.priority}</td>
                  <td className="num">{account.rpm_limit ?? tr("unlimited")}</td>
                  <td className="num">{account.tpm_limit ?? tr("unlimited")}</td>
                  <td>
                    <Badge tone={account.health === 'ok' ? 'ok' : account.health === 'unhealthy' ? 'bad' : 'warn'}>
                      {account.health}
                    </Badge>
                  </td>
                  <td>
                    <TestBadge result={accountTests[account.id]} />
                  </td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void handleTestAccount(account)}
                      >
                        {busy === `account-test-${account.id}` ? tr("testing") : tr("test")}
                      </button>
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void toggleAccountStatus(account)}
                      >
                        {account.status === 'active' ? tr("disable_2") : tr("enable")}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteAccountTarget(account)}>
                        {tr("delete")}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
              {accounts.length === 0 && (
                <tr>
                  <td colSpan={9} className="hint">
                    {tr("no_accounts_yet_accounts_join_routing_once_added")}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <div className="row">
            <h3>{tr("upstream_models")}</h3>
            <span className="sub">{`${models.length} ${tr("items")}`}</span>
          </div>
          <button
            className="btn sm primary"
            type="button"
            onClick={() => {
              setModelDraft(emptyModel)
              setModelErrors({})
              setModelModal(true)
            }}
          >
            {tr("add_upstream_model")}
          </button>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("upstream_model_2")}</th>
                <th>Canonical</th>
                <th>{tr("capabilities")}</th>
                <th>{tr("context")}</th>
                <th>{tr("max_output")}</th>
                <th>{tr("input_output_price")}</th>
                <th className="cell-actions">{tr("actions")}</th>
              </tr>
            </thead>
            <tbody>
              {models.map((model) => (
                <tr key={model.id}>
                  <td>
                    <code className="kbd">{model.name}</code>
                  </td>
                  <td>{model.canonical}</td>
                  <td>{model.capabilities.join(', ') || '—'}</td>
                  <td className="num">{model.context_window ?? '—'}</td>
                  <td className="num">{model.max_output ?? '—'}</td>
                  <td className="num">
                    {fmtPrice(model.price_input)} / {fmtPrice(model.price_output)} {model.currency}
                  </td>
                  <td className="cell-actions">
                    <button className="btn sm danger" type="button" onClick={() => setDeleteModelTarget(model)}>
                      {tr("delete")}
                    </button>
                  </td>
                </tr>
              ))}
              {models.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint">
                    {tr("no_upstream_models_yet_model_groups_reference_upstream_m")}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card" style={{ borderColor: '#f3c7c2' }}>
        <h3>{tr("danger_zone")}</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          {tr("deleting_a_provider_also_removes_its_accounts_and_models")}
        </p>
        <button className="btn danger" type="button" onClick={() => setDeleteProviderOpen(true)} style={{ marginTop: 10 }}>
          {tr("delete_provider_2")}
        </button>
      </div>

      {accountModal && (
        <Modal
          title={tr("add_account")}
          sub={provider.name}
          onClose={() => setAccountModal(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setAccountModal(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="create-account-form" disabled={busy !== null}>
                {busy === 'account-create' ? tr("saving") : tr("save_account")}
              </button>
            </>
          }
        >
          <form id="create-account-form" className="grid" onSubmit={(event) => void handleCreateAccount(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="new-account-name">{tr("account_name")}</label>
                <input
                  id="new-account-name"
                  className="input"
                  value={accountDraft.name}
                  onChange={(event) => setAccountDraft({ ...accountDraft, name: event.target.value })}
                />
                {accountErrors.name && <span className="field-error">{accountErrors.name}</span>}
              </div>
              <div className="field">
                <label htmlFor="new-account-region">{tr("region")}</label>
                <input
                  id="new-account-region"
                  className="input"
                  value={accountDraft.region}
                  onChange={(event) => setAccountDraft({ ...accountDraft, region: event.target.value })}
                />
                {accountErrors.region && <span className="field-error">{accountErrors.region}</span>}
              </div>
            </div>
            <div className="field">
              <label htmlFor="new-account-key">API Key</label>
              <input
                id="new-account-key"
                className="input"
                type="password"
                autoComplete="off"
                value={accountDraft.api_key}
                onChange={(event) => setAccountDraft({ ...accountDraft, api_key: event.target.value })}
              />
              {accountErrors.api_key && <span className="field-error">{accountErrors.api_key}</span>}
            </div>
            <div className="grid cols-4">
              <div className="field">
                <label htmlFor="new-account-weight">{tr("weight")}</label>
                <input
                  id="new-account-weight"
                  className="input"
                  type="number"
                  min={0}
                  max={100}
                  value={accountDraft.weight}
                  onChange={(event) => setAccountDraft({ ...accountDraft, weight: Number(event.target.value) })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-account-priority">{tr("priority")}</label>
                <input
                  id="new-account-priority"
                  className="input"
                  type="number"
                  min={1}
                  max={1000}
                  value={accountDraft.priority}
                  onChange={(event) => setAccountDraft({ ...accountDraft, priority: Number(event.target.value) })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-account-rpm">{tr("rpm_limit")}</label>
                <input
                  id="new-account-rpm"
                  className="input"
                  type="number"
                  value={accountDraft.rpm_limit}
                  onChange={(event) => setAccountDraft({ ...accountDraft, rpm_limit: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-account-tpm">{tr("tpm_limit")}</label>
                <input
                  id="new-account-tpm"
                  className="input"
                  type="number"
                  value={accountDraft.tpm_limit}
                  onChange={(event) => setAccountDraft({ ...accountDraft, tpm_limit: event.target.value })}
                />
              </div>
            </div>
            {actionError && (
              <div className="form-error" role="alert">
                {actionError}
              </div>
            )}
          </form>
        </Modal>
      )}

      {modelModal && (
        <Modal
          title={tr("add_upstream_model")}
          sub={provider.name}
          onClose={() => setModelModal(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModelModal(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="create-model-form" disabled={busy !== null}>
                {busy === 'model-create' ? tr("saving") : tr("save_model")}
              </button>
            </>
          }
        >
          <form id="create-model-form" className="grid" onSubmit={(event) => void handleCreateModel(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="new-model-name">{tr("upstream_model_2")}</label>
                <input
                  id="new-model-name"
                  className="input"
                  placeholder="deepseek-chat"
                  value={modelDraft.name}
                  onChange={(event) => setModelDraft({ ...modelDraft, name: event.target.value })}
                />
                {modelErrors.name && <span className="field-error">{modelErrors.name}</span>}
              </div>
              <div className="field">
                <label htmlFor="new-model-canonical">Canonical</label>
                <input
                  id="new-model-canonical"
                  className="input"
                  placeholder="deepseek-chat"
                  value={modelDraft.canonical}
                  onChange={(event) => setModelDraft({ ...modelDraft, canonical: event.target.value })}
                />
                {modelErrors.canonical && <span className="field-error">{modelErrors.canonical}</span>}
              </div>
            </div>
            <div className="field">
              <label htmlFor="new-model-capabilities">
                {tr("capability_tags_comma_separated")}
              </label>
              <input
                id="new-model-capabilities"
                className="input"
                value={modelDraft.capabilities}
                onChange={(event) => setModelDraft({ ...modelDraft, capabilities: event.target.value })}
              />
              {modelErrors.capabilities && <span className="field-error">{modelErrors.capabilities}</span>}
            </div>
            <div className="grid cols-4">
              <div className="field">
                <label htmlFor="new-model-context">{tr("context_window")}</label>
                <input
                  id="new-model-context"
                  className="input"
                  type="number"
                  value={modelDraft.context_window}
                  onChange={(event) => setModelDraft({ ...modelDraft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-output">{tr("max_output")}</label>
                <input
                  id="new-model-output"
                  className="input"
                  type="number"
                  value={modelDraft.max_output}
                  onChange={(event) => setModelDraft({ ...modelDraft, max_output: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-price-in">{tr("input_price_1m")}</label>
                <input
                  id="new-model-price-in"
                  className="input"
                  type="number"
                  step="0.000001"
                  value={modelDraft.price_input}
                  onChange={(event) => setModelDraft({ ...modelDraft, price_input: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-price-out">{tr("output_price_1m")}</label>
                <input
                  id="new-model-price-out"
                  className="input"
                  type="number"
                  step="0.000001"
                  value={modelDraft.price_output}
                  onChange={(event) => setModelDraft({ ...modelDraft, price_output: event.target.value })}
                />
              </div>
            </div>
            <div className="field" style={{ width: 160 }}>
              <label htmlFor="new-model-currency">{tr("currency")}</label>
              <select
                id="new-model-currency"
                className="select"
                value={modelDraft.currency}
                onChange={(event) => setModelDraft({ ...modelDraft, currency: event.target.value })}
              >
                <option value="CNY">CNY</option>
                <option value="USD">USD</option>
              </select>
            </div>
            {actionError && (
              <div className="form-error" role="alert">
                {actionError}
              </div>
            )}
          </form>
        </Modal>
      )}

      {deleteProviderOpen && (
        <ConfirmDialog
          title={tr("delete_provider")}
          desc={
            <>
              <p>
                {tr("deleting_also_removes_the_accounts_and_models_returns_40")}
              </p>
              <p className="hint">{tr("this_action_is_written_to_the_audit_log")}</p>
            </>
          }
          confirmText={tr("delete")}
          danger
          busy={busy === 'provider-delete'}
          onCancel={() => setDeleteProviderOpen(false)}
          onConfirm={() => void handleDeleteProvider()}
        />
      )}

      {deleteAccountTarget && (
        <ConfirmDialog
          title={tr("delete_account_2", { name: deleteAccountTarget.name })}
          desc={
            <p>
              {tr("returns_409_if_referenced_by_catalog_deployments_deleted")}
            </p>
          }
          confirmText={tr("delete_account")}
          danger
          busy={busy === 'account-delete'}
          onCancel={() => setDeleteAccountTarget(null)}
          onConfirm={() => void handleDeleteAccount()}
        />
      )}

      {deleteModelTarget && (
        <ConfirmDialog
          title={tr("delete_upstream_model", { name: deleteModelTarget.name })}
          desc={
            <p>
              {tr("the_server_returns_409_if_it_is_referenced_by_a_model_al")}
            </p>
          }
          confirmText={tr("delete_model")}
          danger
          busy={busy === 'model-delete'}
          onCancel={() => setDeleteModelTarget(null)}
          onConfirm={() => void handleDeleteModel()}
        />
      )}
    </>
  )
}
