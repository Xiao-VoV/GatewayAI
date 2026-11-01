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
import { useI18n } from '../i18n/I18nProvider'

function groupStatusBadge(status: string, tr: (zh: string, en: string) => string) {
  if (status === 'active') return <Badge tone="ok">{tr('启用', 'Active')}</Badge>
  if (status === 'draft') return <Badge tone="warn">{tr('草稿', 'Draft')}</Badge>
  return <Badge>{tr('已归档', 'Archived')}</Badge>
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
  const { tr } = useI18n()
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
        setActionError(tr('更新模型组失败', 'Failed to update the model group'))
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
        setActionError(tr('添加部署失败', 'Failed to add the deployment'))
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
      handleError(err, tr('更新部署失败', 'Failed to update the deployment'))
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
      handleError(err, tr('删除部署失败', 'Failed to remove the deployment'))
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
        setActionError(tr('保存发布失败', 'Failed to save the publication'))
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
      handleError(err, tr('更新发布状态失败', 'Failed to update publication status'))
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
      handleError(err, tr('删除发布失败', 'Failed to delete the publication'))
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
      handleError(err, tr('归档模型组失败', 'Failed to archive the model group'))
      setArchiveOpen(false)
    } finally {
      setBusy(null)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead
          title={tr('模型组详情', 'Model group detail')}
          actions={<Link className="btn" to="/models">{tr('返回目录', 'Back to catalog')}</Link>}
        />
        <LoadingPanel title={tr('正在读取模型组…', 'Loading model group…')} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead
          title={tr('模型组详情', 'Model group detail')}
          actions={<Link className="btn" to="/models">{tr('返回目录', 'Back to catalog')}</Link>}
        />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  const { group, providers, preview, publications, users, groups } = data

  return (
    <>
      <PageHead
        title={`${tr('模型组', 'Model group')} · ${group.client_model_name}`}
        desc={tr(
          '部署、权重与路由预览；发布/工具默认在后续切片。',
          'Deployments, weights, and route preview; publishing and tool defaults arrive in later slices.',
        )}
        actions={
          <>
            <Link className="btn" to="/models">
              {tr('返回目录', 'Back to catalog')}
            </Link>
            <button className="btn" type="button" onClick={openEdit}>
              {tr('编辑', 'Edit')}
            </button>
            <button className="btn primary" type="button" onClick={openDeployment} disabled={providers.items.length === 0}>
              {tr('添加部署', 'Add deployment')}
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
            <h3>{tr('基础信息', 'Basic info')}</h3>
            {groupStatusBadge(group.status, tr)}
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{tr('对外模型名', 'Client model')}</dt>
            <dd>
              <strong>{group.client_model_name}</strong>
            </dd>
            <dt>{tr('显示名', 'Display name')}</dt>
            <dd>{group.display_name || '—'}</dd>
            <dt>Canonical</dt>
            <dd>{group.canonical}</dd>
            <dt>{tr('能力 / 协议', 'Capabilities / protocols')}</dt>
            <dd>
              {group.capabilities.join(', ') || '—'} / {group.protocols.join(', ') || '—'}
            </dd>
            <dt>{tr('上下文 / 输出', 'Context / output')}</dt>
            <dd>
              {group.context_window ?? '—'} / {group.max_output ?? '—'}
            </dd>
            <dt>{tr('发布', 'Publication')}</dt>
            <dd>
              {group.publications.length === 0 ? (
                <span className="hint">{tr('未发布', 'Not published')}</span>
              ) : (
                group.publications.map((pub) => (
                  <Badge key={pub.id} tone={pub.status === 'active' ? 'ok' : 'warn'}>
                    {pub.status} · {pub.target_count}{tr(' 对象', ' targets')}
                  </Badge>
                ))
              )}
            </dd>
          </dl>
        </div>
        <div className="card">
          <div className="row between">
            <h3>{tr('路由预览', 'Route preview')}</h3>
            <Badge tone={preview.candidates.length > 0 ? 'ok' : 'warn'}>
              {preview.candidates.length}{tr(' 个候选', ' candidates')}
            </Badge>
          </div>
          {preview.candidates.length === 0 ? (
            <p className="sub" style={{ marginTop: 10 }}>
              {tr(
                '没有可用候选；添加启用部署并激活后才会进入路由。',
                'No candidates available. Add an enabled deployment and activate the group to enter routing.',
              )}
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
            <h3>{tr('部署', 'Deployments')}</h3>
            <span className="sub">
              {group.enabled_deployment_count}/{group.deployment_count} {tr('启用', 'enabled')}
            </span>
          </div>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('上游模型', 'Upstream model')}</th>
                <th>{tr('供应商 / 账号', 'Provider / account')}</th>
                <th>{tr('优先级', 'Priority')}</th>
                <th>{tr('权重', 'Weight')}</th>
                <th>{tr('状态', 'Status')}</th>
                <th>{tr('健康', 'Health')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                    <Badge tone={deployment.enabled ? 'ok' : ''}>
                      {deployment.enabled ? tr('启用', 'Enabled') : tr('停用', 'Disabled')}
                    </Badge>
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
                        {deployment.enabled ? tr('停用', 'Disable') : tr('启用', 'Enable')}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteDeploymentTarget(deployment)}>
                        {tr('移除', 'Remove')}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
              {group.deployments.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint">
                    {tr(
                      '还没有部署；添加至少一个启用部署后才能激活/发布。',
                      'No deployments yet. Add at least one enabled deployment before activating or publishing.',
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
            <h3>{tr('发布', 'Publications')}</h3>
            <span className="sub">
              {tr(
                `${publications.length} 条 · 工具默认引用 ${group.tool_default_count}`,
                `${publications.length} publications · ${group.tool_default_count} tool defaults`,
              )}
            </span>
          </div>
          <button
            className="btn sm primary"
            type="button"
            onClick={openCreatePublication}
            disabled={group.status !== 'active' || group.enabled_deployment_count === 0}
          >
            {tr('新建发布', 'New publication')}
          </button>
        </div>
        {group.status !== 'active' && (
          <p className="hint" style={{ marginTop: 6 }}>
            {tr(
              '模型组需先激活且至少 1 个启用部署，才能创建发布。',
              'Activate the model group with at least one enabled deployment before creating a publication.',
            )}
          </p>
        )}
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('状态', 'Status')}</th>
                <th>{tr('发布对象', 'Targets')}</th>
                <th>{tr('可见 / 可用', 'Visible / allowed')}</th>
                <th>{tr('配额', 'Quota')}</th>
                <th>{tr('备注', 'Note')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                        {publication.status === 'active' ? tr('生效', 'Active') : tr('暂停', 'Paused')}
                      </Badge>
                    </td>
                    <td>
                      {publication.targets.length === 0
                        ? '—'
                        : publication.targets
                            .map((target) =>
                              target.subject_type === 'all_users'
                                ? tr('全部用户', 'All users')
                                : target.subject_label ?? target.subject_type,
                            )
                            .join(tr('、', ', '))}
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
                          {publication.status === 'active' ? tr('暂停', 'Pause') : tr('恢复', 'Resume')}
                        </button>
                        <button className="btn sm" type="button" onClick={() => openEditPublication(publication)}>
                          {tr('编辑', 'Edit')}
                        </button>
                        <button className="btn sm danger" type="button" onClick={() => setDeletePublicationTarget(publication)}>
                          {tr('删除', 'Delete')}
                        </button>
                      </span>
                    </td>
                  </tr>
                )
              })}
              {publications.length === 0 && (
                <tr>
                  <td colSpan={6} className="hint">
                    {tr(
                      '尚未发布；发布只对目标用户/用户组/全部用户投影可见目录。',
                      'Not published yet. A publication projects the visible catalog only to the target users, user groups, or all users.',
                    )}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="hint" style={{ marginTop: 8 }}>
          {tr(
            'visible/allowed 强制生效；quota/data_level 目前仅配置展示（config_only）。工具默认与员工预览在后续切片。',
            'visible/allowed are enforced; quota/data_level are configuration-only (config_only) for now. Tool defaults and the employee preview arrive in later slices.',
          )}
        </p>
      </div>

      <div className="card" style={{ borderColor: '#f3c7c2' }}>
        <h3>{tr('危险操作', 'Danger zone')}</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          {tr(
            '归档后模型组不再出现在客户端目录/路由；已发布的组会立即不可见。',
            'Archived model groups no longer appear in the client catalog or routing; published groups become invisible immediately.',
          )}
        </p>
        <button className="btn danger" type="button" onClick={() => setArchiveOpen(true)} style={{ marginTop: 10 }}>
          {tr('归档模型组', 'Archive model group')}
        </button>
      </div>

      {publicationOpen && (
        <Modal
          title={publicationEditId ? tr('编辑发布', 'Edit publication') : tr('新建发布', 'New publication')}
          sub={`${group.client_model_name} · ${tr(
            'visible 是目录可见，allowed 是允许调用',
            'visible controls catalog visibility, allowed controls invocation',
          )}`}
          onClose={() => setPublicationOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setPublicationOpen(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="publication-form" disabled={busy !== null}>
                {busy === 'publication-save' ? tr('保存中…', 'Saving…') : tr('保存发布', 'Save publication')}
              </button>
            </>
          }
        >
          <form id="publication-form" className="grid" onSubmit={(event) => void handleSavePublication(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="publication-status">{tr('状态', 'Status')}</label>
                <select
                  id="publication-status"
                  className="select"
                  value={publicationDraft.status}
                  onChange={(event) =>
                    setPublicationDraft({ ...publicationDraft, status: event.target.value as 'active' | 'paused' })
                  }
                >
                  <option value="active">{tr('生效', 'Active')}</option>
                  <option value="paused">{tr('暂停', 'Paused')}</option>
                </select>
              </div>
              <div className="field">
                <label htmlFor="publication-data-level">{tr('数据等级上限', 'Max data level')}</label>
                <select
                  id="publication-data-level"
                  className="select"
                  value={publicationDraft.data_level_max}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, data_level_max: event.target.value })}
                >
                  <option value="">{tr('未限制', 'Unlimited')}</option>
                  <option value="1">L1</option>
                  <option value="2">L2</option>
                  <option value="3">L3</option>
                  <option value="4">L4</option>
                </select>
              </div>
            </div>

            <div className="field">
              <label>{tr('发布对象', 'Targets')}</label>
              {publicationDraft.targets.map((target, index) => (
                <div key={index} className="card" style={{ boxShadow: 'none', padding: 12, marginBottom: 8 }}>
                  <div className="grid cols-3">
                    <div className="field">
                      <label>{tr('类型', 'Type')}</label>
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
                        <option value="all_users">{tr('全部用户', 'All users')}</option>
                        <option value="user">{tr('指定用户', 'Specific user')}</option>
                        <option value="user_group">{tr('用户组', 'User group')}</option>
                      </select>
                    </div>
                    <div className="field">
                      <label>{tr('对象', 'Target')}</label>
                      {target.subject_type === 'all_users' ? (
                        <input className="input" value={tr('全部用户', 'All users')} disabled />
                      ) : target.subject_type === 'user' ? (
                        <select
                          className="select"
                          value={target.subject_id}
                          onChange={(event) => updatePublicationTarget(index, { subject_id: event.target.value })}
                        >
                          <option value="">{tr('选择用户', 'Select user')}</option>
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
                          <option value="">{tr('选择用户组', 'Select user group')}</option>
                          {groups.map((groupOption) => (
                            <option key={groupOption.id} value={groupOption.id}>
                              {groupOption.name} · {groupOption.member_count}{tr(' 人', ' members')}
                            </option>
                          ))}
                        </select>
                      )}
                    </div>
                    <div className="field">
                      <label>{tr('权限', 'Permissions')}</label>
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
                          {tr('可见', 'Visible')}
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
                          {tr('可用', 'Allowed')}
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
                    {tr('移除对象', 'Remove target')}
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
                {tr('添加对象', 'Add target')}
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
                  placeholder={tr('可选', 'Optional')}
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
                  placeholder={tr('可选', 'Optional')}
                  value={publicationDraft.quota_tpm}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, quota_tpm: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="publication-daily">{tr('quota 日请求', 'Daily request quota')}</label>
                <input
                  id="publication-daily"
                  className="input"
                  type="number"
                  placeholder={tr('可选', 'Optional')}
                  value={publicationDraft.quota_daily_requests}
                  onChange={(event) =>
                    setPublicationDraft({ ...publicationDraft, quota_daily_requests: event.target.value })
                  }
                />
              </div>
            </div>
            <div className="field">
              <label htmlFor="publication-note">{tr('备注', 'Note')}</label>
              <input
                id="publication-note"
                className="input"
                value={publicationDraft.note}
                onChange={(event) => setPublicationDraft({ ...publicationDraft, note: event.target.value })}
              />
            </div>
            <p className="hint">
              {tr(
                'quota 与数据等级当前仅配置（config_only），不在此版本强制执行。',
                'Quota and data level are configuration-only (config_only) and are not enforced in this version.',
              )}
            </p>
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
          title={tr('删除发布？', 'Delete publication?')}
          desc={
            <p>
              {tr(
                '删除后该模型组回到未发布状态，客户端目录/调用立即不可见。操作写入审计。',
                'The model group returns to an unpublished state and immediately disappears from the client catalog and invocation. This action is audited.',
              )}
            </p>
          }
          confirmText={tr('删除', 'Delete')}
          danger
          busy={busy === 'publication-delete'}
          onCancel={() => setDeletePublicationTarget(null)}
          onConfirm={() => void handleDeletePublication()}
        />
      )}

      {editOpen && editDraft && (
        <Modal
          title={tr('编辑模型组', 'Edit model group')}
          onClose={() => setEditOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setEditOpen(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="edit-group-form" disabled={busy !== null}>
                {busy === 'edit' ? tr('保存中…', 'Saving…') : tr('保存', 'Save')}
              </button>
            </>
          }
        >
          <form id="edit-group-form" className="grid" onSubmit={(event) => void handleEdit(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="edit-client-name">{tr('对外模型名', 'Client model')}</label>
                <input
                  id="edit-client-name"
                  className="input"
                  value={editDraft.client_model_name}
                  onChange={(event) => setEditDraft({ ...editDraft, client_model_name: event.target.value })}
                />
                {editErrors.client_model_name && <span className="field-error">{editErrors.client_model_name}</span>}
              </div>
              <div className="field">
                <label htmlFor="edit-display-name">{tr('显示名', 'Display name')}</label>
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
                <label htmlFor="edit-status">{tr('状态', 'Status')}</label>
                <select
                  id="edit-status"
                  className="select"
                  value={editDraft.status}
                  onChange={(event) => setEditDraft({ ...editDraft, status: event.target.value })}
                >
                  <option value="draft">{tr('草稿', 'Draft')}</option>
                  <option value="active">{tr('启用', 'Active')}</option>
                </select>
                <span className="hint">
                  {tr('启用至少需要 1 个启用部署。', 'Activation requires at least one enabled deployment.')}
                </span>
              </div>
              <div className="field">
                <label htmlFor="edit-description">{tr('描述', 'Description')}</label>
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
                <label htmlFor="edit-capabilities">{tr('能力（逗号分隔）', 'Capabilities (comma-separated)')}</label>
                <input
                  id="edit-capabilities"
                  className="input"
                  value={editDraft.capabilities}
                  onChange={(event) => setEditDraft({ ...editDraft, capabilities: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="edit-protocols">{tr('协议（逗号分隔）', 'Protocols (comma-separated)')}</label>
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
                <label htmlFor="edit-context">{tr('上下文窗口', 'Context window')}</label>
                <input
                  id="edit-context"
                  className="input"
                  type="number"
                  value={editDraft.context_window}
                  onChange={(event) => setEditDraft({ ...editDraft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="edit-max-output">{tr('最大输出', 'Max output')}</label>
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
          title={tr('添加部署', 'Add deployment')}
          sub={tr('上游模型 × 渠道账号；同一组内唯一', 'Upstream model × provider account; unique within the group')}
          onClose={() => setDeploymentOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setDeploymentOpen(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button
                className="btn primary"
                type="submit"
                form="create-deployment-form"
                disabled={busy !== null || !deploymentDraft.accountId || !deploymentDraft.upstreamModelId}
              >
                {busy === 'deployment-create' ? tr('保存中…', 'Saving…') : tr('添加部署', 'Add deployment')}
              </button>
            </>
          }
        >
          <form id="create-deployment-form" className="grid" onSubmit={(event) => void handleCreateDeployment(event)} noValidate>
            <div className="field">
              <label htmlFor="deploy-provider">{tr('供应商', 'Provider')}</label>
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
                <label htmlFor="deploy-account">{tr('渠道账号', 'Account')}</label>
                <select
                  id="deploy-account"
                  className="select"
                  value={deploymentDraft.accountId}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, accountId: event.target.value })}
                >
                  <option value="">{tr('选择账号', 'Select account')}</option>
                  {options.data?.accounts.map((account) => (
                    <option key={account.id} value={account.id}>
                      {account.name} · {account.region || '—'} · {account.status}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="deploy-model">{tr('上游模型', 'Upstream model')}</label>
                <select
                  id="deploy-model"
                  className="select"
                  value={deploymentDraft.upstreamModelId}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, upstreamModelId: event.target.value })}
                >
                  <option value="">{tr('选择上游模型', 'Select upstream model')}</option>
                  {options.data?.models.map((model) => (
                    <option key={model.id} value={model.id}>
                      {model.name} · {model.canonical}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            {options.loading && (
              <span className="hint">
                {tr('正在读取供应商账号/模型…', 'Loading provider accounts and models…')}
              </span>
            )}
            {!options.loading && options.data && options.data.accounts.length === 0 && (
              <span className="hint">
                {tr(
                  '该供应商还没有账号；先在供应商详情添加账号。',
                  'This provider has no accounts yet. Add an account in the provider detail page first.',
                )}
              </span>
            )}
            {!options.loading && options.data && options.data.models.length === 0 && (
              <span className="hint">
                {tr(
                  '该供应商还没有上游模型；先在供应商详情添加模型。',
                  'This provider has no upstream models yet. Add a model in the provider detail page first.',
                )}
              </span>
            )}
            <div className="grid cols-3">
              <div className="field">
                <label htmlFor="deploy-priority">{tr('优先级', 'Priority')}</label>
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
                <label htmlFor="deploy-weight">{tr('权重 %', 'Weight %')}</label>
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
                <label htmlFor="deploy-enabled">{tr('状态', 'Status')}</label>
                <select
                  id="deploy-enabled"
                  className="select"
                  value={deploymentDraft.enabled ? '1' : '0'}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, enabled: event.target.value === '1' })}
                >
                  <option value="1">{tr('启用', 'Enabled')}</option>
                  <option value="0">{tr('停用', 'Disabled')}</option>
                </select>
              </div>
            </div>
            <div className="field">
              <label htmlFor="deploy-notes">{tr('备注', 'Note')}</label>
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
          title={tr('归档模型组？', 'Archive model group?')}
          desc={
            <p>
              {tr(
                '归档后不再出现在客户端目录/路由；已发布组会立即不可见。该操作写入审计。',
                'Archived groups no longer appear in the client catalog or routing; published groups become invisible immediately. This action is audited.',
              )}
            </p>
          }
          confirmText={tr('归档', 'Archive')}
          danger
          busy={busy === 'archive'}
          onCancel={() => setArchiveOpen(false)}
          onConfirm={() => void handleArchive()}
        />
      )}

      {deleteDeploymentTarget && (
        <ConfirmDialog
          title={tr('移除部署？', 'Remove deployment?')}
          desc={
            <p>
              {tr(
                '从模型组移除该上游部署；不会删除供应商账号或上游模型。',
                'Removes this upstream deployment from the model group; the provider account and upstream model are not deleted.',
              )}
            </p>
          }
          confirmText={tr('移除', 'Remove')}
          danger
          busy={busy === 'deployment-delete'}
          onCancel={() => setDeleteDeploymentTarget(null)}
          onConfirm={() => void handleDeleteDeployment()}
        />
      )}
    </>
  )
}
