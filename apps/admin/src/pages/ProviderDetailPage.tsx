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
  if (!result) return <span className="hint">{tr('未测试', 'Not tested')}</span>
  if (result.ok)
    return (
      <Badge tone="ok">
        {tr('成功', 'Passed')} · {result.latency_ms}ms
      </Badge>
    )
  return <Badge tone="bad">{result.error?.code ?? tr('失败', 'Failed')}</Badge>
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
      handleApiError(err, tr('测试失败', 'Test failed'))
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
      handleApiError(err, tr('账号测试失败', 'Account test failed'))
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
      handleApiError(err, tr('更新供应商状态失败', 'Failed to update provider status'))
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
      handleApiError(err, tr('更新账号状态失败', 'Failed to update account status'))
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
        setActionError(tr('添加上游账号失败', 'Failed to add account'))
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
        setActionError(tr('添加上游模型失败', 'Failed to add upstream model'))
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
      handleApiError(err, tr('删除供应商失败', 'Failed to delete provider'))
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
      handleApiError(err, tr('删除账号失败', 'Failed to delete account'))
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
      handleApiError(err, tr('删除上游模型失败', 'Failed to delete upstream model'))
    } finally {
      setBusy(null)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead
          title={tr('供应商详情', 'Provider detail')}
          actions={
            <Link className="btn" to="/providers">
              {tr('返回列表', 'Back to list')}
            </Link>
          }
        />
        <LoadingPanel title={tr('正在读取供应商…', 'Loading providers…')} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead
          title={tr('供应商详情', 'Provider detail')}
          actions={
            <Link className="btn" to="/providers">
              {tr('返回列表', 'Back to list')}
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
        title={`${tr('供应商', 'Provider')} · ${provider.name}`}
        desc={tr('账号池、上游模型与连通性。', 'Account pool, upstream models, and connectivity.')}
        actions={
          <>
            <Link className="btn" to="/providers">
              {tr('返回列表', 'Back to list')}
            </Link>
            <button className="btn" type="button" onClick={() => void handleTestProvider()} disabled={busy !== null}>
              {busy === 'provider-test' ? tr('测试中…', 'Testing…') : tr('测试连通性', 'Test connectivity')}
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
            <h3>{tr('基础信息', 'Basics')}</h3>
            <Badge tone={provider.status === 'active' ? 'ok' : ''}>
              {provider.status === 'active' ? tr('启用', 'Enabled') : tr('已禁用', 'Disabled')}
            </Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{tr('类型', 'Type')}</dt>
            <dd>{provider.kind}</dd>
            <dt>Base URL</dt>
            <dd>
              <code className="kbd">{provider.base_url}</code>
            </dd>
            <dt>{tr('描述', 'Description')}</dt>
            <dd>{provider.description || '—'}</dd>
            <dt>{tr('账号', 'Accounts')}</dt>
            <dd>
              {`${provider.account_count} ${tr('个', 'accounts')} · ${provider.active_account_count} ${tr(
                '个启用',
                'active',
              )}`}
            </dd>
            <dt>{tr('上游模型', 'Upstream models')}</dt>
            <dd>{`${provider.upstream_model_count} ${tr('个', 'items')}`}</dd>
            <dt>{tr('引用别名', 'Referenced aliases')}</dt>
            <dd>{`${provider.model_alias_count} ${tr('个', 'aliases')}`}</dd>
            <dt>{tr('可路由', 'Routable')}</dt>
            <dd>
              {provider.routable ? (
                <Badge tone="ok">{tr('是', 'Yes')}</Badge>
              ) : (
                <Badge tone="warn">{tr('否', 'No')}</Badge>
              )}
            </dd>
          </dl>
          <div className="row" style={{ marginTop: 14 }}>
            <button className="btn" type="button" onClick={() => void toggleProviderStatus()} disabled={busy !== null}>
              {provider.status === 'active'
                ? tr('禁用供应商', 'Disable provider')
                : tr('启用供应商', 'Enable provider')}
            </button>
          </div>
        </div>
        <div className="card">
          <h3>{tr('最近测试', 'Last test')}</h3>
          {providerTest ? (
            <div className="statebox" style={{ marginTop: 10 }}>
              <h3>{providerTest.ok ? tr('测试成功', 'Test passed') : tr('测试失败', 'Test failed')}</h3>
              <p>
                {providerTest.ok
                  ? `${tr('耗时', 'Latency')} ${providerTest.latency_ms}ms`
                  : `${providerTest.error?.code ?? tr('失败', 'Failed')} · ${providerTest.error?.message ?? ''}`}
              </p>
              <p className="hint">
                {tr('测试时间', 'Tested at')} {new Date(providerTest.checked_at).toLocaleString(locale)}
              </p>
            </div>
          ) : (
            <p className="sub" style={{ marginTop: 10 }}>
              {tr(
                '点击“测试连通性”检查网络、鉴权与上游响应；不会写入 usage。',
                'Click "Test connectivity" to check network, auth, and upstream response. No usage is recorded.',
              )}
            </p>
          )}
          <dl className="kv" style={{ marginTop: 12 }}>
            <dt>{tr('账号健康', 'Account health')}</dt>
            <dd>{tr('账号健康由测试/上游请求更新', 'Updated by tests and upstream requests')}</dd>
            <dt>{tr('探测范围', 'Probe scope')}</dt>
            <dd>{tr('网络、凭证、模型名、429', 'Network, credentials, model name, 429')}</dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <div className="row">
            <h3>{tr('账号池', 'Account pool')}</h3>
            <span className="sub">{`${accounts.length} ${tr('个账号', 'accounts')}`}</span>
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
            {tr('添加账号', 'Add account')}
          </button>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('账号', 'Account')}</th>
                <th>{tr('区域', 'Region')}</th>
                <th>{tr('权重', 'Weight')}</th>
                <th>{tr('优先级', 'Priority')}</th>
                <th>RPM</th>
                <th>TPM</th>
                <th>{tr('健康', 'Health')}</th>
                <th>{tr('最近测试', 'Last test')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                  <td className="num">{account.rpm_limit ?? tr('不限', 'Unlimited')}</td>
                  <td className="num">{account.tpm_limit ?? tr('不限', 'Unlimited')}</td>
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
                        {busy === `account-test-${account.id}` ? tr('测试中…', 'Testing…') : tr('测试', 'Test')}
                      </button>
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void toggleAccountStatus(account)}
                      >
                        {account.status === 'active' ? tr('禁用', 'Disable') : tr('启用', 'Enable')}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteAccountTarget(account)}>
                        {tr('删除', 'Delete')}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
              {accounts.length === 0 && (
                <tr>
                  <td colSpan={9} className="hint">
                    {tr(
                      '还没有账号；添加后才会参与路由。',
                      'No accounts yet. Accounts join routing once added.',
                    )}
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
            <h3>{tr('上游模型', 'Upstream models')}</h3>
            <span className="sub">{`${models.length} ${tr('个', 'items')}`}</span>
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
            {tr('添加上游模型', 'Add upstream model')}
          </button>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('上游模型名', 'Upstream model')}</th>
                <th>Canonical</th>
                <th>{tr('能力', 'Capabilities')}</th>
                <th>{tr('上下文', 'Context')}</th>
                <th>{tr('最大输出', 'Max output')}</th>
                <th>{tr('输入价 / 输出价', 'Input / output price')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                      {tr('删除', 'Delete')}
                    </button>
                  </td>
                </tr>
              ))}
              {models.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint">
                    {tr(
                      '还没有上游模型；模型组需要引用上游模型与账号部署。',
                      'No upstream models yet. Model groups reference upstream models and account deployments.',
                    )}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card" style={{ borderColor: '#f3c7c2' }}>
        <h3>{tr('危险操作', 'Danger zone')}</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          {tr(
            '删除供应商会级联删除账号与上游模型；被部署/别名引用时服务端返回 409。',
            'Deleting a provider also removes its accounts and models. Returns 409 if referenced by deployments or aliases.',
          )}
        </p>
        <button className="btn danger" type="button" onClick={() => setDeleteProviderOpen(true)} style={{ marginTop: 10 }}>
          {tr('删除供应商', 'Delete provider')}
        </button>
      </div>

      {accountModal && (
        <Modal
          title={tr('添加账号', 'Add account')}
          sub={provider.name}
          onClose={() => setAccountModal(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setAccountModal(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="create-account-form" disabled={busy !== null}>
                {busy === 'account-create' ? tr('保存中…', 'Saving…') : tr('保存账号', 'Save account')}
              </button>
            </>
          }
        >
          <form id="create-account-form" className="grid" onSubmit={(event) => void handleCreateAccount(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="new-account-name">{tr('账号名', 'Account name')}</label>
                <input
                  id="new-account-name"
                  className="input"
                  value={accountDraft.name}
                  onChange={(event) => setAccountDraft({ ...accountDraft, name: event.target.value })}
                />
                {accountErrors.name && <span className="field-error">{accountErrors.name}</span>}
              </div>
              <div className="field">
                <label htmlFor="new-account-region">{tr('区域', 'Region')}</label>
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
                <label htmlFor="new-account-weight">{tr('权重 %', 'Weight %')}</label>
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
                <label htmlFor="new-account-priority">{tr('优先级', 'Priority')}</label>
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
                <label htmlFor="new-account-rpm">{tr('RPM 上限', 'RPM limit')}</label>
                <input
                  id="new-account-rpm"
                  className="input"
                  type="number"
                  value={accountDraft.rpm_limit}
                  onChange={(event) => setAccountDraft({ ...accountDraft, rpm_limit: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-account-tpm">{tr('TPM 上限', 'TPM limit')}</label>
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
          title={tr('添加上游模型', 'Add upstream model')}
          sub={provider.name}
          onClose={() => setModelModal(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModelModal(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="create-model-form" disabled={busy !== null}>
                {busy === 'model-create' ? tr('保存中…', 'Saving…') : tr('保存模型', 'Save model')}
              </button>
            </>
          }
        >
          <form id="create-model-form" className="grid" onSubmit={(event) => void handleCreateModel(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="new-model-name">{tr('上游模型名', 'Upstream model')}</label>
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
                {tr('能力标签（逗号分隔）', 'Capability tags (comma separated)')}
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
                <label htmlFor="new-model-context">{tr('上下文窗口', 'Context window')}</label>
                <input
                  id="new-model-context"
                  className="input"
                  type="number"
                  value={modelDraft.context_window}
                  onChange={(event) => setModelDraft({ ...modelDraft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-output">{tr('最大输出', 'Max output')}</label>
                <input
                  id="new-model-output"
                  className="input"
                  type="number"
                  value={modelDraft.max_output}
                  onChange={(event) => setModelDraft({ ...modelDraft, max_output: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-price-in">{tr('输入价 / 1M', 'Input price / 1M')}</label>
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
                <label htmlFor="new-model-price-out">{tr('输出价 / 1M', 'Output price / 1M')}</label>
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
              <label htmlFor="new-model-currency">{tr('币种', 'Currency')}</label>
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
          title={tr('删除供应商？', 'Delete provider?')}
          desc={
            <>
              <p>
                {tr(
                  '删除会同时移除账号与上游模型；被部署/别名引用时服务端会返回 409。',
                  'Deleting also removes the accounts and models. Returns 409 if referenced by deployments or aliases.',
                )}
              </p>
              <p className="hint">{tr('该操作写入审计。', 'This action is written to the audit log.')}</p>
            </>
          }
          confirmText={tr('删除', 'Delete')}
          danger
          busy={busy === 'provider-delete'}
          onCancel={() => setDeleteProviderOpen(false)}
          onConfirm={() => void handleDeleteProvider()}
        />
      )}

      {deleteAccountTarget && (
        <ConfirmDialog
          title={tr(
            `删除账号 ${deleteAccountTarget.name}？`,
            `Delete account ${deleteAccountTarget.name}?`,
          )}
          desc={
            <p>
              {tr(
                '被 catalog 部署引用时服务端会返回 409；删除后该账号不再参与路由。',
                'Returns 409 if referenced by catalog deployments. Deleted accounts stop routing.',
              )}
            </p>
          }
          confirmText={tr('删除账号', 'Delete account')}
          danger
          busy={busy === 'account-delete'}
          onCancel={() => setDeleteAccountTarget(null)}
          onConfirm={() => void handleDeleteAccount()}
        />
      )}

      {deleteModelTarget && (
        <ConfirmDialog
          title={tr(
            `删除上游模型 ${deleteModelTarget.name}？`,
            `Delete upstream model ${deleteModelTarget.name}?`,
          )}
          desc={
            <p>
              {tr(
                '被模型别名或 catalog 部署引用时服务端会返回 409。',
                'The server returns 409 if it is referenced by a model alias or catalog deployment.',
              )}
            </p>
          }
          confirmText={tr('删除模型', 'Delete model')}
          danger
          busy={busy === 'model-delete'}
          onCancel={() => setDeleteModelTarget(null)}
          onConfirm={() => void handleDeleteModel()}
        />
      )}
    </>
  )
}
