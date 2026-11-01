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

function fmtPrice(value: number | null) {
  return value === null ? '—' : String(value)
}

function TestBadge({ result }: { result: TestResult | undefined }) {
  if (!result) return <span className="hint">未测试</span>
  if (result.ok) return <Badge tone="ok">成功 · {result.latency_ms}ms</Badge>
  return <Badge tone="bad">{result.error?.code ?? '失败'}</Badge>
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
      handleApiError(err, '测试失败')
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
      handleApiError(err, '账号测试失败')
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
      handleApiError(err, '更新供应商状态失败')
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
      handleApiError(err, '更新账号状态失败')
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
        setActionError('添加上游账号失败')
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
        setActionError('添加上游模型失败')
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
      handleApiError(err, '删除供应商失败')
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
      handleApiError(err, '删除账号失败')
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
      handleApiError(err, '删除上游模型失败')
    } finally {
      setBusy(null)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead title="供应商详情" actions={<Link className="btn" to="/providers">返回列表</Link>} />
        <LoadingPanel title="正在读取供应商…" />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead title="供应商详情" actions={<Link className="btn" to="/providers">返回列表</Link>} />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  const { provider, accounts, models } = data

  return (
    <>
      <PageHead
        title={`供应商 · ${provider.name}`}
        desc="账号池、上游模型与连通性。"
        actions={
          <>
            <Link className="btn" to="/providers">
              返回列表
            </Link>
            <button className="btn" type="button" onClick={() => void handleTestProvider()} disabled={busy !== null}>
              {busy === 'provider-test' ? '测试中…' : '测试连通性'}
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
            <h3>基础信息</h3>
            <Badge tone={provider.status === 'active' ? 'ok' : ''}>
              {provider.status === 'active' ? '启用' : '已禁用'}
            </Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>类型</dt>
            <dd>{provider.kind}</dd>
            <dt>Base URL</dt>
            <dd>
              <code className="kbd">{provider.base_url}</code>
            </dd>
            <dt>描述</dt>
            <dd>{provider.description || '—'}</dd>
            <dt>账号</dt>
            <dd>
              {provider.account_count} 个 · {provider.active_account_count} 个启用
            </dd>
            <dt>上游模型</dt>
            <dd>{provider.upstream_model_count} 个</dd>
            <dt>引用别名</dt>
            <dd>{provider.model_alias_count} 个</dd>
            <dt>可路由</dt>
            <dd>{provider.routable ? <Badge tone="ok">是</Badge> : <Badge tone="warn">否</Badge>}</dd>
          </dl>
          <div className="row" style={{ marginTop: 14 }}>
            <button className="btn" type="button" onClick={() => void toggleProviderStatus()} disabled={busy !== null}>
              {provider.status === 'active' ? '禁用供应商' : '启用供应商'}
            </button>
          </div>
        </div>
        <div className="card">
          <h3>最近测试</h3>
          {providerTest ? (
            <div className="statebox" style={{ marginTop: 10 }}>
              <h3>{providerTest.ok ? '测试成功' : '测试失败'}</h3>
              <p>
                {providerTest.ok
                  ? `耗时 ${providerTest.latency_ms}ms`
                  : `${providerTest.error?.code ?? '失败'} · ${providerTest.error?.message ?? ''}`}
              </p>
              <p className="hint">测试时间 {new Date(providerTest.checked_at).toLocaleString('zh-CN')}</p>
            </div>
          ) : (
            <p className="sub" style={{ marginTop: 10 }}>
              点击“测试连通性”检查网络、鉴权与上游响应；不会写入 usage。
            </p>
          )}
          <dl className="kv" style={{ marginTop: 12 }}>
            <dt>账号健康</dt>
            <dd>账号健康由测试/上游请求更新</dd>
            <dt>探测范围</dt>
            <dd>网络、凭证、模型名、429</dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <div className="row">
            <h3>账号池</h3>
            <span className="sub">{accounts.length} 个账号</span>
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
            添加账号
          </button>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>账号</th>
                <th>区域</th>
                <th>权重</th>
                <th>优先级</th>
                <th>RPM</th>
                <th>TPM</th>
                <th>健康</th>
                <th>最近测试</th>
                <th className="cell-actions">操作</th>
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
                  <td className="num">{account.rpm_limit ?? '不限'}</td>
                  <td className="num">{account.tpm_limit ?? '不限'}</td>
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
                        {busy === `account-test-${account.id}` ? '测试中…' : '测试'}
                      </button>
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void toggleAccountStatus(account)}
                      >
                        {account.status === 'active' ? '禁用' : '启用'}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteAccountTarget(account)}>
                        删除
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
              {accounts.length === 0 && (
                <tr>
                  <td colSpan={9} className="hint">
                    还没有账号；添加后才会参与路由。
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
            <h3>上游模型</h3>
            <span className="sub">{models.length} 个</span>
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
            添加上游模型
          </button>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>上游模型名</th>
                <th>Canonical</th>
                <th>能力</th>
                <th>上下文</th>
                <th>最大输出</th>
                <th>输入价 / 输出价</th>
                <th className="cell-actions">操作</th>
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
                      删除
                    </button>
                  </td>
                </tr>
              ))}
              {models.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint">
                    还没有上游模型；模型组需要引用上游模型与账号部署。
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>

      <div className="card" style={{ borderColor: '#f3c7c2' }}>
        <h3>危险操作</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          删除供应商会级联删除账号与上游模型；被部署/别名引用时服务端返回 409。
        </p>
        <button className="btn danger" type="button" onClick={() => setDeleteProviderOpen(true)} style={{ marginTop: 10 }}>
          删除供应商
        </button>
      </div>

      {accountModal && (
        <Modal
          title="添加账号"
          sub={provider.name}
          onClose={() => setAccountModal(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setAccountModal(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="create-account-form" disabled={busy !== null}>
                {busy === 'account-create' ? '保存中…' : '保存账号'}
              </button>
            </>
          }
        >
          <form id="create-account-form" className="grid" onSubmit={(event) => void handleCreateAccount(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="new-account-name">账号名</label>
                <input
                  id="new-account-name"
                  className="input"
                  value={accountDraft.name}
                  onChange={(event) => setAccountDraft({ ...accountDraft, name: event.target.value })}
                />
                {accountErrors.name && <span className="field-error">{accountErrors.name}</span>}
              </div>
              <div className="field">
                <label htmlFor="new-account-region">区域</label>
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
                <label htmlFor="new-account-weight">权重 %</label>
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
                <label htmlFor="new-account-priority">优先级</label>
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
                <label htmlFor="new-account-rpm">RPM 上限</label>
                <input
                  id="new-account-rpm"
                  className="input"
                  type="number"
                  value={accountDraft.rpm_limit}
                  onChange={(event) => setAccountDraft({ ...accountDraft, rpm_limit: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-account-tpm">TPM 上限</label>
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
          title="添加上游模型"
          sub={provider.name}
          onClose={() => setModelModal(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModelModal(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="create-model-form" disabled={busy !== null}>
                {busy === 'model-create' ? '保存中…' : '保存模型'}
              </button>
            </>
          }
        >
          <form id="create-model-form" className="grid" onSubmit={(event) => void handleCreateModel(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="new-model-name">上游模型名</label>
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
              <label htmlFor="new-model-capabilities">能力标签（逗号分隔）</label>
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
                <label htmlFor="new-model-context">上下文窗口</label>
                <input
                  id="new-model-context"
                  className="input"
                  type="number"
                  value={modelDraft.context_window}
                  onChange={(event) => setModelDraft({ ...modelDraft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-output">最大输出</label>
                <input
                  id="new-model-output"
                  className="input"
                  type="number"
                  value={modelDraft.max_output}
                  onChange={(event) => setModelDraft({ ...modelDraft, max_output: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="new-model-price-in">输入价 / 1M</label>
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
                <label htmlFor="new-model-price-out">输出价 / 1M</label>
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
              <label htmlFor="new-model-currency">币种</label>
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
          title="删除供应商？"
          desc={
            <>
              <p>删除会同时移除账号与上游模型；被部署/别名引用时服务端会返回 409。</p>
              <p className="hint">该操作写入审计。</p>
            </>
          }
          confirmText="删除"
          danger
          busy={busy === 'provider-delete'}
          onCancel={() => setDeleteProviderOpen(false)}
          onConfirm={() => void handleDeleteProvider()}
        />
      )}

      {deleteAccountTarget && (
        <ConfirmDialog
          title={`删除账号 ${deleteAccountTarget.name}？`}
          desc={<p>被 catalog 部署引用时服务端会返回 409；删除后该账号不再参与路由。</p>}
          confirmText="删除账号"
          danger
          busy={busy === 'account-delete'}
          onCancel={() => setDeleteAccountTarget(null)}
          onConfirm={() => void handleDeleteAccount()}
        />
      )}

      {deleteModelTarget && (
        <ConfirmDialog
          title={`删除上游模型 ${deleteModelTarget.name}？`}
          desc={<p>被模型别名或 catalog 部署引用时服务端会返回 409。</p>}
          confirmText="删除模型"
          danger
          busy={busy === 'model-delete'}
          onCancel={() => setDeleteModelTarget(null)}
          onConfirm={() => void handleDeleteModel()}
        />
      )}
    </>
  )
}
