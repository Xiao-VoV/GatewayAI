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
import type { Translate } from '../i18n/translate'

function groupStatusBadge(status: string, tr: Translate) {
  if (status === 'active') return <Badge tone="ok">{tr("active_2")}</Badge>
  if (status === 'draft') return <Badge tone="warn">{tr("draft")}</Badge>
  return <Badge>{tr("archived")}</Badge>
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
        setActionError(tr("failed_to_update_the_model_group"))
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
        setActionError(tr("failed_to_add_the_deployment"))
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
      handleError(err, tr("failed_to_update_the_deployment"))
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
      handleError(err, tr("failed_to_remove_the_deployment"))
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
        setActionError(tr("failed_to_save_the_publication"))
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
      handleError(err, tr("failed_to_update_publication_status"))
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
      handleError(err, tr("failed_to_delete_the_publication"))
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
      handleError(err, tr("failed_to_archive_the_model_group"))
      setArchiveOpen(false)
    } finally {
      setBusy(null)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead
          title={tr("model_group_detail")}
          actions={<Link className="btn" to="/models">{tr("back_to_catalog")}</Link>}
        />
        <LoadingPanel title={tr("loading_model_group")} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead
          title={tr("model_group_detail")}
          actions={<Link className="btn" to="/models">{tr("back_to_catalog")}</Link>}
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
        title={`${tr("model_group")} · ${group.client_model_name}`}
        desc={tr("deployments_weights_and_route_preview_publishing_and_too")}
        actions={
          <>
            <Link className="btn" to="/models">
              {tr("back_to_catalog")}
            </Link>
            <button className="btn" type="button" onClick={openEdit}>
              {tr("edit")}
            </button>
            <button className="btn primary" type="button" onClick={openDeployment} disabled={providers.items.length === 0}>
              {tr("add_deployment")}
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
            <h3>{tr("basic_info")}</h3>
            {groupStatusBadge(group.status, tr)}
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{tr("client_model")}</dt>
            <dd>
              <strong>{group.client_model_name}</strong>
            </dd>
            <dt>{tr("display_name")}</dt>
            <dd>{group.display_name || '—'}</dd>
            <dt>Canonical</dt>
            <dd>{group.canonical}</dd>
            <dt>{tr("capabilities_protocols")}</dt>
            <dd>
              {group.capabilities.join(', ') || '—'} / {group.protocols.join(', ') || '—'}
            </dd>
            <dt>{tr("context_output")}</dt>
            <dd>
              {group.context_window ?? '—'} / {group.max_output ?? '—'}
            </dd>
            <dt>{tr("publication")}</dt>
            <dd>
              {group.publications.length === 0 ? (
                <span className="hint">{tr("not_published")}</span>
              ) : (
                group.publications.map((pub) => (
                  <Badge key={pub.id} tone={pub.status === 'active' ? 'ok' : 'warn'}>
                    {pub.status} · {pub.target_count}{tr("targets_2")}
                  </Badge>
                ))
              )}
            </dd>
          </dl>
        </div>
        <div className="card">
          <div className="row between">
            <h3>{tr("route_preview")}</h3>
            <Badge tone={preview.candidates.length > 0 ? 'ok' : 'warn'}>
              {preview.candidates.length}{tr("candidates")}
            </Badge>
          </div>
          {preview.candidates.length === 0 ? (
            <p className="sub" style={{ marginTop: 10 }}>
              {tr("no_candidates_available_add_an_enabled_deployment_and_ac")}
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
            <h3>{tr("deployments")}</h3>
            <span className="sub">
              {group.enabled_deployment_count}/{group.deployment_count} {tr("enabled_2")}
            </span>
          </div>
        </div>
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("upstream_model")}</th>
                <th>{tr("provider_account")}</th>
                <th>{tr("priority")}</th>
                <th>{tr("weight_2")}</th>
                <th>{tr("status")}</th>
                <th>{tr("health")}</th>
                <th className="cell-actions">{tr("actions")}</th>
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
                      {deployment.enabled ? tr("enabled") : tr("disabled_3")}
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
                        {deployment.enabled ? tr("disable") : tr("enable")}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteDeploymentTarget(deployment)}>
                        {tr("remove")}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
              {group.deployments.length === 0 && (
                <tr>
                  <td colSpan={7} className="hint">
                    {tr("no_deployments_yet_add_at_least_one_enabled_deployment_b")}
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
            <h3>{tr("publications")}</h3>
            <span className="sub">
              {tr("publications_tool_defaults", { length: publications.length, tool_default_count: group.tool_default_count })}
            </span>
          </div>
          <button
            className="btn sm primary"
            type="button"
            onClick={openCreatePublication}
            disabled={group.status !== 'active' || group.enabled_deployment_count === 0}
          >
            {tr("new_publication")}
          </button>
        </div>
        {group.status !== 'active' && (
          <p className="hint" style={{ marginTop: 6 }}>
            {tr("activate_the_model_group_with_at_least_one_enabled_deplo")}
          </p>
        )}
        <div className="table-wrap" style={{ marginTop: 10 }}>
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("status")}</th>
                <th>{tr("targets")}</th>
                <th>{tr("visible_allowed")}</th>
                <th>{tr("quota")}</th>
                <th>{tr("note")}</th>
                <th className="cell-actions">{tr("actions")}</th>
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
                        {publication.status === 'active' ? tr("active_3") : tr("paused")}
                      </Badge>
                    </td>
                    <td>
                      {publication.targets.length === 0
                        ? '—'
                        : publication.targets
                            .map((target) =>
                              target.subject_type === 'all_users'
                                ? tr("all_users")
                                : target.subject_label ?? target.subject_type,
                            )
                            .join(tr("msg"))}
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
                          {publication.status === 'active' ? tr("pause") : tr("resume")}
                        </button>
                        <button className="btn sm" type="button" onClick={() => openEditPublication(publication)}>
                          {tr("edit")}
                        </button>
                        <button className="btn sm danger" type="button" onClick={() => setDeletePublicationTarget(publication)}>
                          {tr("delete")}
                        </button>
                      </span>
                    </td>
                  </tr>
                )
              })}
              {publications.length === 0 && (
                <tr>
                  <td colSpan={6} className="hint">
                    {tr("not_published_yet_a_publication_projects_the_visible_cat")}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <p className="hint" style={{ marginTop: 8 }}>
          {tr("visible_allowed_are_enforced_quota_data_level_are_config")}
        </p>
      </div>

      <div className="card" style={{ borderColor: '#f3c7c2' }}>
        <h3>{tr("danger_zone")}</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          {tr("archived_model_groups_no_longer_appear_in_the_client_cat")}
        </p>
        <button className="btn danger" type="button" onClick={() => setArchiveOpen(true)} style={{ marginTop: 10 }}>
          {tr("archive_model_group_2")}
        </button>
      </div>

      {publicationOpen && (
        <Modal
          title={publicationEditId ? tr("edit_publication") : tr("new_publication")}
          sub={`${group.client_model_name} · ${tr("visible_controls_catalog_visibility_allowed_controls_inv")}`}
          onClose={() => setPublicationOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setPublicationOpen(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="publication-form" disabled={busy !== null}>
                {busy === 'publication-save' ? tr("saving") : tr("save_publication")}
              </button>
            </>
          }
        >
          <form id="publication-form" className="grid" onSubmit={(event) => void handleSavePublication(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="publication-status">{tr("status")}</label>
                <select
                  id="publication-status"
                  className="select"
                  value={publicationDraft.status}
                  onChange={(event) =>
                    setPublicationDraft({ ...publicationDraft, status: event.target.value as 'active' | 'paused' })
                  }
                >
                  <option value="active">{tr("active_3")}</option>
                  <option value="paused">{tr("paused")}</option>
                </select>
              </div>
              <div className="field">
                <label htmlFor="publication-data-level">{tr("max_data_level")}</label>
                <select
                  id="publication-data-level"
                  className="select"
                  value={publicationDraft.data_level_max}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, data_level_max: event.target.value })}
                >
                  <option value="">{tr("unlimited_2")}</option>
                  <option value="1">L1</option>
                  <option value="2">L2</option>
                  <option value="3">L3</option>
                  <option value="4">L4</option>
                </select>
              </div>
            </div>

            <div className="field">
              <label>{tr("targets")}</label>
              {publicationDraft.targets.map((target, index) => (
                <div key={index} className="card" style={{ boxShadow: 'none', padding: 12, marginBottom: 8 }}>
                  <div className="grid cols-3">
                    <div className="field">
                      <label>{tr("type")}</label>
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
                        <option value="all_users">{tr("all_users")}</option>
                        <option value="user">{tr("specific_user")}</option>
                        <option value="user_group">{tr("user_group")}</option>
                      </select>
                    </div>
                    <div className="field">
                      <label>{tr("target")}</label>
                      {target.subject_type === 'all_users' ? (
                        <input className="input" value={tr("all_users")} disabled />
                      ) : target.subject_type === 'user' ? (
                        <select
                          className="select"
                          value={target.subject_id}
                          onChange={(event) => updatePublicationTarget(index, { subject_id: event.target.value })}
                        >
                          <option value="">{tr("select_user")}</option>
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
                          <option value="">{tr("select_user_group")}</option>
                          {groups.map((groupOption) => (
                            <option key={groupOption.id} value={groupOption.id}>
                              {groupOption.name} · {groupOption.member_count}{tr("members_3")}
                            </option>
                          ))}
                        </select>
                      )}
                    </div>
                    <div className="field">
                      <label>{tr("permissions")}</label>
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
                          {tr("visible")}
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
                          {tr("allowed")}
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
                    {tr("remove_target")}
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
                {tr("add_target")}
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
                  placeholder={tr("optional")}
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
                  placeholder={tr("optional")}
                  value={publicationDraft.quota_tpm}
                  onChange={(event) => setPublicationDraft({ ...publicationDraft, quota_tpm: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="publication-daily">{tr("daily_request_quota")}</label>
                <input
                  id="publication-daily"
                  className="input"
                  type="number"
                  placeholder={tr("optional")}
                  value={publicationDraft.quota_daily_requests}
                  onChange={(event) =>
                    setPublicationDraft({ ...publicationDraft, quota_daily_requests: event.target.value })
                  }
                />
              </div>
            </div>
            <div className="field">
              <label htmlFor="publication-note">{tr("note")}</label>
              <input
                id="publication-note"
                className="input"
                value={publicationDraft.note}
                onChange={(event) => setPublicationDraft({ ...publicationDraft, note: event.target.value })}
              />
            </div>
            <p className="hint">
              {tr("quota_and_data_level_are_configuration_only_config_only")}
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
          title={tr("delete_publication")}
          desc={
            <p>
              {tr("the_model_group_returns_to_an_unpublished_state_and_imme")}
            </p>
          }
          confirmText={tr("delete")}
          danger
          busy={busy === 'publication-delete'}
          onCancel={() => setDeletePublicationTarget(null)}
          onConfirm={() => void handleDeletePublication()}
        />
      )}

      {editOpen && editDraft && (
        <Modal
          title={tr("edit_model_group")}
          onClose={() => setEditOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setEditOpen(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="edit-group-form" disabled={busy !== null}>
                {busy === 'edit' ? tr("saving") : tr("save")}
              </button>
            </>
          }
        >
          <form id="edit-group-form" className="grid" onSubmit={(event) => void handleEdit(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="edit-client-name">{tr("client_model")}</label>
                <input
                  id="edit-client-name"
                  className="input"
                  value={editDraft.client_model_name}
                  onChange={(event) => setEditDraft({ ...editDraft, client_model_name: event.target.value })}
                />
                {editErrors.client_model_name && <span className="field-error">{editErrors.client_model_name}</span>}
              </div>
              <div className="field">
                <label htmlFor="edit-display-name">{tr("display_name")}</label>
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
                <label htmlFor="edit-status">{tr("status")}</label>
                <select
                  id="edit-status"
                  className="select"
                  value={editDraft.status}
                  onChange={(event) => setEditDraft({ ...editDraft, status: event.target.value })}
                >
                  <option value="draft">{tr("draft")}</option>
                  <option value="active">{tr("active_2")}</option>
                </select>
                <span className="hint">
                  {tr("activation_requires_at_least_one_enabled_deployment")}
                </span>
              </div>
              <div className="field">
                <label htmlFor="edit-description">{tr("description")}</label>
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
                <label htmlFor="edit-capabilities">{tr("capabilities_comma_separated")}</label>
                <input
                  id="edit-capabilities"
                  className="input"
                  value={editDraft.capabilities}
                  onChange={(event) => setEditDraft({ ...editDraft, capabilities: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="edit-protocols">{tr("protocols_comma_separated")}</label>
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
                <label htmlFor="edit-context">{tr("context_window")}</label>
                <input
                  id="edit-context"
                  className="input"
                  type="number"
                  value={editDraft.context_window}
                  onChange={(event) => setEditDraft({ ...editDraft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="edit-max-output">{tr("max_output")}</label>
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
          title={tr("add_deployment")}
          sub={tr("upstream_model_provider_account_unique_within_the_group")}
          onClose={() => setDeploymentOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setDeploymentOpen(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button
                className="btn primary"
                type="submit"
                form="create-deployment-form"
                disabled={busy !== null || !deploymentDraft.accountId || !deploymentDraft.upstreamModelId}
              >
                {busy === 'deployment-create' ? tr("saving") : tr("add_deployment")}
              </button>
            </>
          }
        >
          <form id="create-deployment-form" className="grid" onSubmit={(event) => void handleCreateDeployment(event)} noValidate>
            <div className="field">
              <label htmlFor="deploy-provider">{tr("provider")}</label>
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
                <label htmlFor="deploy-account">{tr("account")}</label>
                <select
                  id="deploy-account"
                  className="select"
                  value={deploymentDraft.accountId}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, accountId: event.target.value })}
                >
                  <option value="">{tr("select_account")}</option>
                  {options.data?.accounts.map((account) => (
                    <option key={account.id} value={account.id}>
                      {account.name} · {account.region || '—'} · {account.status}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <label htmlFor="deploy-model">{tr("upstream_model")}</label>
                <select
                  id="deploy-model"
                  className="select"
                  value={deploymentDraft.upstreamModelId}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, upstreamModelId: event.target.value })}
                >
                  <option value="">{tr("select_upstream_model")}</option>
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
                {tr("loading_provider_accounts_and_models")}
              </span>
            )}
            {!options.loading && options.data && options.data.accounts.length === 0 && (
              <span className="hint">
                {tr("this_provider_has_no_accounts_yet_add_an_account_in_the")}
              </span>
            )}
            {!options.loading && options.data && options.data.models.length === 0 && (
              <span className="hint">
                {tr("this_provider_has_no_upstream_models_yet_add_a_model_in")}
              </span>
            )}
            <div className="grid cols-3">
              <div className="field">
                <label htmlFor="deploy-priority">{tr("priority")}</label>
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
                <label htmlFor="deploy-weight">{tr("weight")}</label>
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
                <label htmlFor="deploy-enabled">{tr("status")}</label>
                <select
                  id="deploy-enabled"
                  className="select"
                  value={deploymentDraft.enabled ? '1' : '0'}
                  onChange={(event) => setDeploymentDraft({ ...deploymentDraft, enabled: event.target.value === '1' })}
                >
                  <option value="1">{tr("enabled")}</option>
                  <option value="0">{tr("disabled_3")}</option>
                </select>
              </div>
            </div>
            <div className="field">
              <label htmlFor="deploy-notes">{tr("note")}</label>
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
          title={tr("archive_model_group")}
          desc={
            <p>
              {tr("archived_groups_no_longer_appear_in_the_client_catalog_o")}
            </p>
          }
          confirmText={tr("archive")}
          danger
          busy={busy === 'archive'}
          onCancel={() => setArchiveOpen(false)}
          onConfirm={() => void handleArchive()}
        />
      )}

      {deleteDeploymentTarget && (
        <ConfirmDialog
          title={tr("remove_deployment")}
          desc={
            <p>
              {tr("removes_this_upstream_deployment_from_the_model_group_th")}
            </p>
          }
          confirmText={tr("remove")}
          danger
          busy={busy === 'deployment-delete'}
          onCancel={() => setDeleteDeploymentTarget(null)}
          onConfirm={() => void handleDeleteDeployment()}
        />
      )}
    </>
  )
}
