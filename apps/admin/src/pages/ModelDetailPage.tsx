import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { ApiError } from '../api/client'
import {
  archiveModelGroup,
  createDeployment,
  deleteDeployment,
  getModelGroup,
  patchDeployment,
  patchModelGroup,
  previewModelGroupRoutes,
} from '../api/catalogApi'
import type { DeploymentAdmin, ModelGroupPatchInput, RoutePreview } from '../api/catalog'
import { listPublications, createPublication, patchPublication, deletePublication } from '../api/publicationsApi'
import type {
  Publication,
  PublicationCreateInput,
  PublicationTarget,
  PublicationTargetInput,
  PublicationUpdateInput,
} from '../api/publications'
import { listUserGroups, listUsers } from '../api/accessApi'
import type { AdminUser, UserGroupSummary } from '../api/access'
import { listAccounts, listModels, listProviders } from '../api/providersApi'
import type { ModelList, ProviderAccount } from '../api/providers'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { ConfirmDialog, Modal } from '../components/Modal'

function groupStatusBadge(status: string) {
  if (status === 'active') return <Badge tone="ok">启用</Badge>
  if (status === 'draft') return <Badge tone="warn">草稿</Badge>
  return <Badge>已归档</Badge>
}

interface EditDraft {
  client_model_name: string
  display_name: string
  canonical: string
  description: string
  capabilities: string
  context_window: string
  max_output: string
  protocols: string
  status: string
}

interface DeploymentDraft {
  providerId: string
  accountId: string
  upstreamModelId: string
  priority: number
  weight: number
  enabled: boolean
  notes: string
}

interface PublicationTargetDraft {
  subject_type: 'all_users' | 'user' | 'user_group'
  subject_id: string
  visible: boolean
  allowed: boolean
}

interface PublicationDraft {
  status: 'active' | 'paused'
  data_level_max: string
  quota_rpm: string
  quota_tpm: string
  quota_daily_requests: string
  note: string
  targets: PublicationTargetDraft[]
}

const emptyPublication: PublicationDraft = {
  status: 'active',
  data_level_max: '',
  quota_rpm: '',
  quota_tpm: '',
  quota_daily_requests: '',
  note: '',
  targets: [{ subject_type: 'all_users', subject_id: '', visible: true, allowed: true }],
}

const emptyDeployment: DeploymentDraft = {
  providerId: '',
  accountId: '',
  upstreamModelId: '',
  priority: 1,
  weight: 100,
  enabled: true,
  notes: '',
}

export function ModelDetailPage() {
  const { groupId = '' } = useParams()
  const navigate = useNavigate()
  const { data, loading, error, reload } = useAsync(
    () =>
      Promise.all([
        getModelGroup(groupId),
        listProviders({ limit: 100 }),
        previewModelGroupRoutes(groupId),
        listPublications(groupId),
        listUsers(200),
        listUserGroups(200),
      ]).then(
        ([group, providers, preview, publications, users, groups]) => ({
          group,
          providers,
          preview,
          publications: publications.items,
          users: users.items,
          groups: groups.items,
        }),
      ),
    [groupId],
  )

  const [editOpen, setEditOpen] = useState(false)
  const [editDraft, setEditDraft] = useState<EditDraft | null>(null)
  const [editErrors, setEditErrors] = useState<Record<string, string>>({})
  const [deploymentOpen, setDeploymentOpen] = useState(false)
  const [deploymentDraft, setDeploymentDraft] = useState<DeploymentDraft>(emptyDeployment)
  const [deploymentErrors, setDeploymentErrors] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [archiveOpen, setArchiveOpen] = useState(false)
  const [deleteDeploymentTarget, setDeleteDeploymentTarget] = useState<DeploymentAdmin | null>(null)
  const [publicationOpen, setPublicationOpen] = useState(false)
  const [publicationEditId, setPublicationEditId] = useState<string | null>(null)
  const [publicationDraft, setPublicationDraft] = useState<PublicationDraft>(emptyPublication)
  const [publicationErrors, setPublicationErrors] = useState<Record<string, string>>({})
  const [deletePublicationTarget, setDeletePublicationTarget] = useState<Publication | null>(null)

  const selectedProviderId = deploymentDraft.providerId
  const options = useAsync(
    () =>
      selectedProviderId
        ? Promise.all([listAccounts(selectedProviderId), listModels(selectedProviderId)]).then(([accounts, models]) => ({
            accounts: accounts.items,
            models: models.items,
          }))
        : Promise.resolve({ accounts: [] as ProviderAccount[], models: [] as ModelList['items'] }),
    [selectedProviderId],
  )

  useEffect(() => {
    if (!data) return
    if (!deploymentDraft.providerId && data.providers.items.length > 0) {
      setDeploymentDraft((draft) => ({ ...draft, providerId: data.providers.items[0].id }))
    }
  }, [data, deploymentDraft.providerId])

  function handleError(err: unknown, fallback: string) {
    setActionError(err instanceof ApiError ? err.detail : fallback)
  }

  function openEdit() {
    if (!data) return
    setEditDraft({
      client_model_name: data.group.client_model_name,
      display_name: data.group.display_name,
      canonical: data.group.canonical,
      description: data.group.description,
      capabilities: data.group.capabilities.join(', '),
      context_window: data.group.context_window === null ? '' : String(data.group.context_window),
      max_output: data.group.max_output === null ? '' : String(data.group.max_output),
      protocols: data.group.protocols.join(', '),
      status: data.group.status === 'archived' ? 'draft' : data.group.status,
    })
    setEditErrors({})
    setEditOpen(true)
  }

  async function handleEdit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!editDraft) return
    setBusy('edit')
    setEditErrors({})
    setActionError(null)
    const patch: ModelGroupPatchInput = {
      client_model_name: editDraft.client_model_name.trim(),
      display_name: editDraft.display_name.trim(),
      canonical: editDraft.canonical.trim(),
      description: editDraft.description.trim(),
      capabilities: editDraft.capabilities
        .split(',')
        .map((value) => value.trim())
        .filter(Boolean),
      context_window: editDraft.context_window.trim() ? Number(editDraft.context_window) : null,
      max_output: editDraft.max_output.trim() ? Number(editDraft.max_output) : null,
      protocols: editDraft.protocols
        .split(',')
        .map((value) => value.trim())
        .filter(Boolean),
      status: editDraft.status,
    }
    try {
      await patchModelGroup(groupId, patch)
      setEditOpen(false)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setEditErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError('更新模型组失败')
      }
    } finally {
      setBusy(null)
    }
  }

  function openDeployment() {
    setDeploymentDraft({
      ...emptyDeployment,
      providerId: data?.providers.items[0]?.id ?? '',
    })
    setDeploymentErrors({})
    setDeploymentOpen(true)
  }

  async function handleCreateDeployment(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('deployment-create')
    setDeploymentErrors({})
    setActionError(null)
    try {
      await createDeployment({
        upstream_model_id: deploymentDraft.upstreamModelId,
        provider_account_id: deploymentDraft.accountId,
        model_group_id: groupId,
        priority: deploymentDraft.priority,
        weight: deploymentDraft.weight,
        enabled: deploymentDraft.enabled,
        notes: deploymentDraft.notes.trim(),
      })
      setDeploymentOpen(false)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setDeploymentErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError('添加部署失败')
      }
    } finally {
      setBusy(null)
    }
  }

  async function toggleDeployment(deployment: DeploymentAdmin) {
    setBusy(`deployment-${deployment.id}`)
    setActionError(null)
    try {
      await patchDeployment(deployment.id, { enabled: !deployment.enabled })
      reload()
    } catch (err) {
      handleError(err, '更新部署失败')
    } finally {
      setBusy(null)
    }
  }

  async function handleDeleteDeployment() {
    if (!deleteDeploymentTarget) return
    setBusy('deployment-delete')
    setActionError(null)
    try {
      await deleteDeployment(deleteDeploymentTarget.id)
      setDeleteDeploymentTarget(null)
      reload()
    } catch (err) {
      handleError(err, '删除部署失败')
    } finally {
      setBusy(null)
    }
  }

  function openCreatePublication() {
    setPublicationEditId(null)
    setPublicationDraft(emptyPublication)
    setPublicationErrors({})
    setPublicationOpen(true)
  }

  function openEditPublication(publication: Publication) {
    setPublicationEditId(publication.id)
    setPublicationDraft({
      status: publication.status,
      data_level_max: publication.data_level_max === null ? '' : String(publication.data_level_max),
      quota_rpm: publication.quota_rpm === null ? '' : String(publication.quota_rpm),
      quota_tpm: publication.quota_tpm === null ? '' : String(publication.quota_tpm),
      quota_daily_requests:
        publication.quota_daily_requests === null ? '' : String(publication.quota_daily_requests),
      note: publication.note,
      targets: publication.targets.map((target) => ({
        subject_type: target.subject_type === 'application' || target.subject_type === 'device' ? 'all_users' : target.subject_type,
        subject_id: target.subject_id ?? '',
        visible: target.visible,
        allowed: target.allowed,
      })),
    })
    setPublicationErrors({})
    setPublicationOpen(true)
  }

  function updatePublicationTarget(index: number, patch: Partial<PublicationTargetDraft>) {
    setPublicationDraft((draft) => ({
      ...draft,
      targets: draft.targets.map((target, targetIndex) =>
        targetIndex === index ? { ...target, ...patch } : target,
      ),
    }))
  }

  async function handleSavePublication(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('publication-save')
    setPublicationErrors({})
    setActionError(null)
    const targets: PublicationTargetInput[] = publicationDraft.targets.map((target) => ({
      subject_type: target.subject_type,
      subject_id: target.subject_type === 'all_users' ? null : target.subject_id || null,
      visible: target.visible,
      allowed: target.allowed,
    }))
    const payload = {
      status: publicationDraft.status,
      data_level_max: publicationDraft.data_level_max ? Number(publicationDraft.data_level_max) : null,
      quota_rpm: publicationDraft.quota_rpm ? Number(publicationDraft.quota_rpm) : null,
      quota_tpm: publicationDraft.quota_tpm ? Number(publicationDraft.quota_tpm) : null,
      quota_daily_requests: publicationDraft.quota_daily_requests
        ? Number(publicationDraft.quota_daily_requests)
        : null,
      note: publicationDraft.note,
      targets,
    }
    try {
      if (publicationEditId) {
        await patchPublication(publicationEditId, payload as PublicationUpdateInput)
      } else {
        await createPublication({ model_group_id: groupId, ...payload } as PublicationCreateInput)
      }
      setPublicationOpen(false)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setPublicationErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError('保存发布失败')
      }
    } finally {
      setBusy(null)
    }
  }

  async function togglePublication(publication: Publication) {
    setBusy(`publication-${publication.id}`)
    setActionError(null)
    try {
      await patchPublication(publication.id, {
        status: publication.status === 'active' ? 'paused' : 'active',
      })
      reload()
    } catch (err) {
      handleError(err, '更新发布状态失败')
    } finally {
      setBusy(null)
    }
  }

  async function handleDeletePublication() {
    if (!deletePublicationTarget) return
    setBusy('publication-delete')
    setActionError(null)
    try {
      await deletePublication(deletePublicationTarget.id)
      setDeletePublicationTarget(null)
      reload()
    } catch (err) {
      handleError(err, '删除发布失败')
    } finally {
      setBusy(null)
    }
  }

  async function handleArchive() {
    setBusy('archive')
    setActionError(null)
    try {
      await archiveModelGroup(groupId)
      navigate('/models', { replace: true })
    } catch (err) {
      handleError(err, '归档模型组失败')
      setArchiveOpen(false)
    } finally {
      setBusy(null)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead title="模型组详情" actions={<Link className="btn" to="/models">返回目录</Link>} />
        <LoadingPanel title="正在读取模型组…" />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead title="模型组详情" actions={<Link className="btn" to="/models">返回目录</Link>} />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  const { group, providers, preview, publications, users, groups } = data

  return (
    <>
      <PageHead
        title={`模型组 · ${group.client_model_name}`}
        desc="部署、权重与路由预览；发布/工具默认在后续切片。"
        actions={
          <>
            <Link className="btn" to="/models">
              返回目录
            </Link>
            <button className="btn" type="button" onClick={openEdit}>
              编辑
            </button>
            <button className="btn primary" type="button" onClick={openDeployment} disabled={providers.items.length === 0}>
              添加部署
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
            {groupStatusBadge(group.status)}
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>对外模型名</dt>
            <dd>
              <strong>{group.client_model_name}</strong>
            </dd>
            <dt>显示名</dt>
            <dd>{group.display_name || '—'}</dd>
            <dt>Canonical</dt>
            <dd>{group.canonical}</dd>
            <dt>能力 / 协议</dt>
            <dd>
              {group.capabilities.join(', ') || '—'} / {group.protocols.join(', ') || '—'}
            </dd>
            <dt>上下文 / 输出</dt>
            <dd>
              {group.context_window ?? '—'} / {group.max_output ?? '—'}
            </dd>
            <dt>发布</dt>
            <dd>
              {group.publications.length === 0 ? (
                <span className="hint">未发布</span>
              ) : (
                group.publications.map((pub) => (
                  <Badge key={pub.id} tone={pub.status === 'active' ? 'ok' : 'warn'}>
                    {pub.status} · {pub.target_count} 对象
                  </Badge>
                ))
              )}
            </dd>
          </dl>
        </div>
        <div className="card">
          <div className="row between">
            <h3>路由预览</h3>
            <Badge tone={preview.candidates.length > 0 ? 'ok' : 'warn'}>
              {preview.candidates.length} 个候选
            </Badge>
          </div>
          {preview.candidates.length === 0 ? (
            <p className="sub" style={{ marginTop: 10 }}>
              没有可用候选；添加启用部署并激活后才会进入路由。
            </p>
          ) : (
            <div style={{ marginTop: 10 }}>
              {preview.candidates.map((candidate) => (
                <div className="status-row" key={candidate.deployment_id}>
                  <span>
                    #{candidate.priority} {candidate.upstream_model} · {candidate.provider_name}/{candidate.account_name}
                  </span>
                  <span style={{ marginLeft: 'auto' }}>{candidate.weight}%</span>
                </div>
              ))}
            </div>
          )}
          {preview.warnings.length > 0 && (
            <p className="note warn" style={{ marginTop: 10 }}>
              {preview.warnings.join(' · ')}
            </p>
          )}
        </div>
      </div>

      <div className="card">
        <div className="row between">
          <div className="row">
            <h3>部署</h3>
            <span className="sub">
              {group.enabled_deployment_count}/{group.deployment_count} 启用
            </span>
          </div>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>上游模型</th>
                <th>供应商 / 账号</th>
                <th>优先级</th>
                <th>权重</th>
                <th>状态</th>
                <th>健康</th>
                <th className="cell-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {group.deployments.map((deployment) => (
                <tr key={deployment.id} className={deployment.enabled ? '' : 'is-off'}>
                  <td>
                    <code className="kbd">{deployment.upstream_model}</code>
                    <div className="hint">{deployment.canonical}</div>
                  </td>
                  <td>
                    {deployment.provider_name}
                    <div className="hint">
                      {deployment.account_name} · {deployment.account_region || '—'}
                    </div>
                  </td>
                  <td className="num">{deployment.priority}</td>
                  <td className="num">{deployment.weight}%</td>
                  <td>
                    <Badge tone={deployment.enabled ? 'ok' : ''}>{deployment.enabled ? '启用' : '停用'}</Badge>
                  </td>
                  <td>
                    <Badge tone={deployment.provider_status === 'active' ? 'ok' : 'warn'}>
                      {deployment.provider_status}
                    </Badge>
                  </td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void toggleDeployment(deployment)}
                      >
                        {deployment.enabled ? '停用' : '启用'}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteDeploymentTarget(deployment)}>
                        移除
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
              {group.deployments.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint">
                    还没有部署；添加至少一个启用部署后才能激活/发布。
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
            <h3>发布</h3>
            <span className="sub">{publications.length} 条 · 工具默认引用 {group.tool_default_count}</span>
          </div>
          <button
            className="btn sm primary"
            type="button"
            onClick={openCreatePublication}
            disabled={group.status !== 'active' || group.enabled_deployment_count === 0}
          >
            新建发布
          </button>
        </div>
        {group.status !== 'active' && (
          <p className="hint" style={{ marginTop: 6 }}>
            模型组需先激活且至少 1 个启用部署，才能创建发布。
          </p>
        )}
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>状态</th>
                <th>发布对象</th>
                <th>可见 / 可用</th>
                <th>配额</th>
                <th>备注</th>
                <th className="cell-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {publications.map((publication) => {
                const visible = publication.targets.filter((target) => target.visible).length
                const allowed = publication.targets.filter((target) => target.allowed).length
                return (
                  <tr key={publication.id}>
                    <td>
                      <Badge tone={publication.status === 'active' ? 'ok' : 'warn'}>
                        {publication.status === 'active' ? '生效' : '暂停'}
                      </Badge>
                    </td>
                    <td>
                      {publication.targets.length === 0
                        ? '—'
                        : publication.targets
                            .map((target) =>
                              target.subject_type === 'all_users'
                                ? '全部用户'
                                : target.subject_label ?? target.subject_type,
                            )
                            .join('、')}
                    </td>
                    <td className="num">
                      {visible} / {allowed}
                    </td>
                    <td className="num">
                      RPM {publication.quota_rpm ?? '—'} · TPM {publication.quota_tpm ?? '—'}
                    </td>
                    <td>{publication.note || '—'}</td>
                    <td className="cell-actions">
                      <span className="row-actions">
                        <button
                          className="btn sm"
                          type="button"
                          onClick={() => void togglePublication(publication)}
                          disabled={busy !== null}
                        >
                          {publication.status === 'active' ? '暂停' : '恢复'}
                        </button>
                        <button className="btn sm" type="button" onClick={() => openEditPublication(publication)}>
                          编辑
                        </button>
                        <button className="btn sm danger" type="button" onClick={() => setDeletePublicationTarget(publication)}>
                          删除
                        </button>
                      </span>
                    </td>
                  </tr>
                )
              })}
              {publications.length === 0 && (
                <tr>
                  <td colSpan={6} className="hint">
                    尚未发布；发布只对目标用户/用户组/全部用户投影可见目录。
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="hint" style={{ marginTop: 8 }}>
          visible/allowed 强制生效；quota/data_level 目前仅配置展示（config_only）。工具默认与员工预览在后续切片。
        </p>
      </div>

      <div className="card" style={{ borderColor: '#f3c7c2' }}>
        <h3>危险操作</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          归档后模型组不再出现在客户端目录/路由；已发布的组会立即不可见。
        </p>
        <button className="btn danger" type="button" onClick={() => setArchiveOpen(true)} style={{ marginTop: 10 }}>
          归档模型组
        </button>
      </div>

      {publicationOpen && (
        <Modal
          title={publicationEditId ? '编辑发布' : '新建发布'}
          sub={`${group.client_model_name} · visible 是目录可见，allowed 是允许调用`}
          onClose={() => setPublicationOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setPublicationOpen(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="publication-form" disabled={busy !== null}>
                {busy === 'publication-save' ? '保存中…' : '保存发布'}
              </button>
            </>
          }
        >
          <form id="publication-form" className="grid" onSubmit={(event) => void handleSavePublication(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="publication-status">状态</label>
                <select
                  id="publication-status"
                  className="select"
                  value={publicationDraft.status}
                  onChange={(event) =>
                    setPublicationDraft({ ...publicationDraft, status: event.target.value as 'active' | 'paused' })
                  }
                >
                  <option value="active">生效</option>
                  <option value="paused">暂停</option>
                </select>
              </div>
              <div className="field">
                <label htmlFor="publication-data-level">数据等级上限</label>
                <select
                  id="publication-data-level"
                  className="select"
                  value={publicationDraft.data_level_max}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, data_level_max: event.target.value })}
                >
                  <option value="">未限制</option>
                  <option value="1">L1</option>
                  <option value="2">L2</option>
                  <option value="3">L3</option>
                  <option value="4">L4</option>
                </select>
              </div>
            </div>

            <div className="field">
              <label>发布对象</label>
              {publicationDraft.targets.map((target, index) => (
                <div key={index} className="card" style={{ boxShadow: 'none', padding: 12, marginBottom: 8 }}>
                  <div className="grid cols-3">
                    <div className="field">
                      <label>类型</label>
                      <select
                        className="select"
                        value={target.subject_type}
                        onChange={(event) =>
                          updatePublicationTarget(index, {
                            subject_type: event.target.value as PublicationTargetDraft['subject_type'],
                            subject_id: '',
                          })
                        }
                      >
                        <option value="all_users">全部用户</option>
                        <option value="user">指定用户</option>
                        <option value="user_group">用户组</option>
                      </select>
                    </div>
                    <div className="field">
                      <label>对象</label>
                      {target.subject_type === 'all_users' ? (
                        <input className="input" value="全部用户" disabled />
                      ) : target.subject_type === 'user' ? (
                        <select
                          className="select"
                          value={target.subject_id}
                          onChange={(event) => updatePublicationTarget(index, { subject_id: event.target.value })}
                        >
                          <option value="">选择用户</option>
                          {users.map((user) => (
                            <option key={user.id} value={user.id}>
                              {user.display_name || user.email} · {user.email}
                            </option>
                          ))}
                        </select>
                      ) : (
                        <select
                          className="select"
                          value={target.subject_id}
                          onChange={(event) => updatePublicationTarget(index, { subject_id: event.target.value })}
                        >
                          <option value="">选择用户组</option>
                          {groups.map((groupOption) => (
                            <option key={groupOption.id} value={groupOption.id}>
                              {groupOption.name} · {groupOption.member_count} 人
                            </option>
                          ))}
                        </select>
                      )}
                    </div>
                    <div className="field">
                      <label>权限</label>
                      <div className="row" style={{ height: 32 }}>
                        <label className="chk">
                          <input
                            type="checkbox"
                            checked={target.visible}
                            onChange={(event) =>
                              updatePublicationTarget(index, {
                                visible: event.target.checked,
                                allowed: event.target.checked ? target.allowed : false,
                              })
                            }
                          />
                          可见
                        </label>
                        <label className="chk">
                          <input
                            type="checkbox"
                            checked={target.allowed}
                            onChange={(event) =>
                              updatePublicationTarget(index, {
                                allowed: event.target.checked,
                                visible: event.target.checked ? true : target.visible,
                              })
                            }
                          />
                          可用
                        </label>
                      </div>
                    </div>
                  </div>
                  <button
                    className="btn sm danger"
                    type="button"
                    onClick={() =>
                      setPublicationDraft((draft) => ({
                        ...draft,
                        targets: draft.targets.filter((_, targetIndex) => targetIndex !== index),
                      }))
                    }
                    disabled={publicationDraft.targets.length === 1}
                  >
                    移除对象
                  </button>
                </div>
              ))}
              <button
                className="btn sm"
                type="button"
                onClick={() =>
                  setPublicationDraft((draft) => ({
                    ...draft,
                    targets: [
                      ...draft.targets,
                      { subject_type: 'user', subject_id: '', visible: true, allowed: true },
                    ],
                  }))
                }
              >
                添加对象
              </button>
              {publicationErrors.targets && <span className="field-error">{publicationErrors.targets}</span>}
            </div>

            <div className="grid cols-3">
              <div className="field">
                <label htmlFor="publication-rpm">quota RPM</label>
                <input
                  id="publication-rpm"
                  className="input"
                  type="number"
                  placeholder="可选"
                  value={publicationDraft.quota_rpm}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, quota_rpm: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="publication-tpm">quota TPM</label>
                <input
                  id="publication-tpm"
                  className="input"
                  type="number"
                  placeholder="可选"
                  value={publicationDraft.quota_tpm}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, quota_tpm: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="publication-daily">quota 日请求</label>
                <input
                  id="publication-daily"
                  className="input"
                  type="number"
                  placeholder="可选"
                  value={publicationDraft.quota_daily_requests}
                  onChange={(event) =>
                    setPublicationDraft({ ...publicationDraft, quota_daily_requests: event.target.value })
                  }
                />
              </div>
            </div>
            <div className="field">
              <label htmlFor="publication-note">备注</label>
              <input
                id="publication-note"
                className="input"
                value={publicationDraft.note}
                onChange={(event) => setPublicationDraft({ ...publicationDraft, note: event.target.value })}
              />
            </div>
            <p className="hint">quota 与数据等级当前仅配置（config_only），不在此版本强制执行。</p>
            {actionError && (
              <div className="form-error" role="alert">
                {actionError}
              </div>
            )}
          </form>
        </Modal>
      )}

      {deletePublicationTarget && (
        <ConfirmDialog
          title="删除发布？"
          desc={<p>删除后该模型组回到未发布状态，客户端目录/调用立即不可见。操作写入审计。</p>}
          confirmText="删除"
          danger
          busy={busy === 'publication-delete'}
          onCancel={() => setDeletePublicationTarget(null)}
          onConfirm={() => void handleDeletePublication()}
        />
      )}

      {editOpen && editDraft && (
        <Modal
          title="编辑模型组"
          onClose={() => setEditOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setEditOpen(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="edit-group-form" disabled={busy !== null}>
                {busy === 'edit' ? '保存中…' : '保存'}
              </button>
            </>
          }
        >
          <form id="edit-group-form" className="grid" onSubmit={(event) => void handleEdit(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="edit-client-name">对外模型名</label>
                <input
                  id="edit-client-name"
                  className="input"
                  value={editDraft.client_model_name}
                  onChange={(event) => setEditDraft({ ...editDraft, client_model_name: event.target.value })}
                />
                {editErrors.client_model_name && <span className="field-error">{editErrors.client_model_name}</span>}
              </div>
              <div className="field">
                <label htmlFor="edit-display-name">显示名</label>
                <input
                  id="edit-display-name"
                  className="input"
                  value={editDraft.display_name}
                  onChange={(event) => setEditDraft({ ...editDraft, display_name: event.target.value })}
                />
              </div>
            </div>
            <div className="field">
              <label htmlFor="edit-canonical">Canonical</label>
              <input
                id="edit-canonical"
                className="input"
                value={editDraft.canonical}
                onChange={(event) => setEditDraft({ ...editDraft, canonical: event.target.value })}
              />
              {editErrors.canonical && <span className="field-error">{editErrors.canonical}</span>}
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="edit-status">状态</label>
                <select
                  id="edit-status"
                  className="select"
                  value={editDraft.status}
                  onChange={(event) => setEditDraft({ ...editDraft, status: event.target.value })}
                >
                  <option value="draft">草稿</option>
                  <option value="active">启用</option>
                </select>
                <span className="hint">启用至少需要 1 个启用部署。</span>
              </div>
              <div className="field">
                <label htmlFor="edit-description">描述</label>
                <input
                  id="edit-description"
                  className="input"
                  value={editDraft.description}
                  onChange={(event) => setEditDraft({ ...editDraft, description: event.target.value })}
                />
              </div>
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="edit-capabilities">能力（逗号分隔）</label>
                <input
                  id="edit-capabilities"
                  className="input"
                  value={editDraft.capabilities}
                  onChange={(event) => setEditDraft({ ...editDraft, capabilities: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="edit-protocols">协议（逗号分隔）</label>
                <input
                  id="edit-protocols"
                  className="input"
                  value={editDraft.protocols}
                  onChange={(event) => setEditDraft({ ...editDraft, protocols: event.target.value })}
                />
              </div>
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="edit-context">上下文窗口</label>
                <input
                  id="edit-context"
                  className="input"
                  type="number"
                  value={editDraft.context_window}
                  onChange={(event) => setEditDraft({ ...editDraft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="edit-max-output">最大输出</label>
                <input
                  id="edit-max-output"
                  className="input"
                  type="number"
                  value={editDraft.max_output}
                  onChange={(event) => setEditDraft({ ...editDraft, max_output: event.target.value })}
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

      {deploymentOpen && (
        <Modal
          title="添加部署"
          sub="上游模型 × 渠道账号；同一组内唯一"
          onClose={() => setDeploymentOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setDeploymentOpen(false)} disabled={busy !== null}>
                取消
              </button>
              <button
                className="btn primary"
                type="submit"
                form="create-deployment-form"
                disabled={busy !== null || !deploymentDraft.accountId || !deploymentDraft.upstreamModelId}
              >
                {busy === 'deployment-create' ? '保存中…' : '添加部署'}
              </button>
            </>
          }
        >
          <form id="create-deployment-form" className="grid" onSubmit={(event) => void handleCreateDeployment(event)} noValidate>
            <div className="field">
              <label htmlFor="deploy-provider">供应商</label>
              <select
                id="deploy-provider"
                className="select"
                value={deploymentDraft.providerId}
                onChange={(event) =>
                  setDeploymentDraft({
                    ...deploymentDraft,
                    providerId: event.target.value,
                    accountId: '',
                    upstreamModelId: '',
                  })
                }
              >
                {providers.items.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.name}
                  </option>
                ))}
              </select>
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="deploy-account">渠道账号</label>
                <select
                  id="deploy-account"
                  className="select"
                  value={deploymentDraft.accountId}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, accountId: event.target.value })}
                >
                  <option value="">选择账号</option>
                  {options.data?.accounts.map((account) => (
                    <option key={account.id} value={account.id}>
                      {account.name} · {account.region || '—'} · {account.status}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="deploy-model">上游模型</label>
                <select
                  id="deploy-model"
                  className="select"
                  value={deploymentDraft.upstreamModelId}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, upstreamModelId: event.target.value })}
                >
                  <option value="">选择上游模型</option>
                  {options.data?.models.map((model) => (
                    <option key={model.id} value={model.id}>
                      {model.name} · {model.canonical}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            {options.loading && <span className="hint">正在读取供应商账号/模型…</span>}
            {!options.loading && options.data && options.data.accounts.length === 0 && (
              <span className="hint">该供应商还没有账号；先在供应商详情添加账号。</span>
            )}
            {!options.loading && options.data && options.data.models.length === 0 && (
              <span className="hint">该供应商还没有上游模型；先在供应商详情添加模型。</span>
            )}
            <div className="grid cols-3">
              <div className="field">
                <label htmlFor="deploy-priority">优先级</label>
                <input
                  id="deploy-priority"
                  className="input"
                  type="number"
                  min={1}
                  value={deploymentDraft.priority}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, priority: Number(event.target.value) })}
                />
              </div>
              <div className="field">
                <label htmlFor="deploy-weight">权重 %</label>
                <input
                  id="deploy-weight"
                  className="input"
                  type="number"
                  min={1}
                  max={100}
                  value={deploymentDraft.weight}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, weight: Number(event.target.value) })}
                />
              </div>
              <div className="field">
                <label htmlFor="deploy-enabled">状态</label>
                <select
                  id="deploy-enabled"
                  className="select"
                  value={deploymentDraft.enabled ? '1' : '0'}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, enabled: event.target.value === '1' })}
                >
                  <option value="1">启用</option>
                  <option value="0">停用</option>
                </select>
              </div>
            </div>
            <div className="field">
              <label htmlFor="deploy-notes">备注</label>
              <input
                id="deploy-notes"
                className="input"
                value={deploymentDraft.notes}
                onChange={(event) => setDeploymentDraft({ ...deploymentDraft, notes: event.target.value })}
              />
            </div>
            {deploymentErrors.upstream_model_id && (
              <span className="field-error">{deploymentErrors.upstream_model_id}</span>
            )}
            {deploymentErrors.provider_account_id && (
              <span className="field-error">{deploymentErrors.provider_account_id}</span>
            )}
            {actionError && (
              <div className="form-error" role="alert">
                {actionError}
              </div>
            )}
          </form>
        </Modal>
      )}

      {archiveOpen && (
        <ConfirmDialog
          title="归档模型组？"
          desc={<p>归档后不再出现在客户端目录/路由；已发布组会立即不可见。该操作写入审计。</p>}
          confirmText="归档"
          danger
          busy={busy === 'archive'}
          onCancel={() => setArchiveOpen(false)}
          onConfirm={() => void handleArchive()}
        />
      )}

      {deleteDeploymentTarget && (
        <ConfirmDialog
          title="移除部署？"
          desc={<p>从模型组移除该上游部署；不会删除供应商账号或上游模型。</p>}
          confirmText="移除"
          danger
          busy={busy === 'deployment-delete'}
          onCancel={() => setDeleteDeploymentTarget(null)}
          onConfirm={() => void handleDeleteDeployment()}
        />
      )}
    </>
  )
}
