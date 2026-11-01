import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import { listModelGroups } from '../api/catalogApi'
import { listUserGroups, listUsers } from '../api/accessApi'
import type { AdminUser, UserGroupSummary } from '../api/access'
import type { ModelGroupSummary } from '../api/catalog'
import {
  createToolDefault,
  deleteToolDefault,
  listToolDefaults,
  updateToolDefault,
} from '../api/toolDefaults'
import type { ToolDefaultCreateInput, ToolModelDefault, ToolDefaultUpdateInput } from '../api/toolDefaults'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ConfirmDialog, Modal } from '../components/Modal'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { useI18n } from '../i18n/I18nProvider'
import type { Translate } from '../i18n/translate'

interface Draft {
  tool_key: string
  scope_type: 'global' | 'user_group' | 'user'
  scope_id: string
  default_model_group_id: string
  allow_switch: boolean
  allowed_model_group_ids: string[]
}

const emptyDraft: Draft = {
  tool_key: 'claude_code',
  scope_type: 'global',
  scope_id: '',
  default_model_group_id: '',
  allow_switch: true,
  allowed_model_group_ids: [],
}

function scopeLabel(defaultItem: ToolModelDefault, tr: Translate) {
  if (defaultItem.scope_type === 'global') return tr("global")
  return `${defaultItem.scope_type === 'user_group' ? tr("user_group") : tr("user")} · ${defaultItem.scope_label ?? defaultItem.scope_id?.slice(0, 8) ?? '—'}`
}

export function ToolDefaultsPage() {
  const { tr } = useI18n()
  const [scopeFilter, setScopeFilter] = useState('')
  const { data, loading, error, reload } = useAsync(
    () =>
      Promise.all([
        listToolDefaults({ scope_type: scopeFilter || undefined, limit: 200 }),
        listModelGroups({ status: 'active', limit: 200 }),
        listUsers(200),
        listUserGroups(200),
      ]).then(([defaults, groups, users, userGroups]) => ({
        defaults: defaults.items,
        groups: groups.items,
        users: users.items,
        userGroups: userGroups.items,
      })),
    [scopeFilter],
  )

  const [modalOpen, setModalOpen] = useState(false)
  const [editId, setEditId] = useState<string | null>(null)
  const [draft, setDraft] = useState<Draft>(emptyDraft)
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [actionError, setActionError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<ToolModelDefault | null>(null)

  function openCreate() {
    setEditId(null)
    setDraft({
      ...emptyDraft,
      default_model_group_id: data?.groups[0]?.id ?? '',
      allowed_model_group_ids: data?.groups[1] ? [data.groups[1].id] : [],
    })
    setErrors({})
    setActionError(null)
    setModalOpen(true)
  }

  function openEdit(item: ToolModelDefault) {
    setEditId(item.id)
    setDraft({
      tool_key: item.tool_key,
      scope_type: item.scope_type,
      scope_id: item.scope_id ?? '',
      default_model_group_id: item.default_model_group_id,
      allow_switch: item.allow_switch,
      allowed_model_group_ids: item.allowed_model_groups.map((group) => group.id),
    })
    setErrors({})
    setActionError(null)
    setModalOpen(true)
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('save')
    setErrors({})
    setActionError(null)
    const allowed = draft.allowed_model_group_ids.filter((id) => id !== draft.default_model_group_id)
    try {
      if (editId) {
        const patch: ToolDefaultUpdateInput = {
          default_model_group_id: draft.default_model_group_id,
          allow_switch: draft.allow_switch,
          allowed_model_group_ids: allowed,
        }
        await updateToolDefault(editId, patch)
      } else {
        const payload: ToolDefaultCreateInput = {
          tool_key: draft.tool_key.trim(),
          scope_type: draft.scope_type,
          scope_id: draft.scope_type === 'global' ? null : draft.scope_id || null,
          default_model_group_id: draft.default_model_group_id,
          allow_switch: draft.allow_switch,
          allowed_model_group_ids: allowed,
        }
        await createToolDefault(payload)
      }
      setModalOpen(false)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError(tr("failed_to_save_tool_default"))
      }
    } finally {
      setBusy(null)
    }
  }

  async function handleDelete() {
    if (!deleteTarget) return
    setBusy('delete')
    setActionError(null)
    try {
      await deleteToolDefault(deleteTarget.id)
      setDeleteTarget(null)
      reload()
    } catch (err) {
      setActionError(err instanceof ApiError ? err.detail : tr("failed_to_delete_tool_default"))
    } finally {
      setBusy(null)
    }
  }

  function toggleWhitelist(groupId: string) {
    setDraft((current) => ({
      ...current,
      allowed_model_group_ids: current.allowed_model_group_ids.includes(groupId)
        ? current.allowed_model_group_ids.filter((id) => id !== groupId)
        : [...current.allowed_model_group_ids, groupId],
    }))
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr("tool_defaults")}
        desc={tr("default_model_per_tool_and_switchable_allowlist_user_use")}
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr("new_tool_default")}
            </button>
          </>
        }
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 180 }}>
            <label htmlFor="tool-scope-filter">{tr("scope")}</label>
            <select
              id="tool-scope-filter"
              className="select"
              value={scopeFilter}
              onChange={(event) => setScopeFilter(event.target.value)}
            >
              <option value="">{tr("all")}</option>
              <option value="global">{tr("global")}</option>
              <option value="user_group">{tr("user_group")}</option>
              <option value="user">{tr("user")}</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            {tr("the_allowlist_only_narrows_publishing_the_effective_clie")}
          </span>
        </div>
      </div>

      {loading && !data ? (
        <LoadingPanel title={tr("loading_tool_defaults")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.defaults.length === 0 ? (
        <EmptyPanel
          title={tr("no_tool_defaults_yet")}
          desc={tr("configure_default_models_and_switchable_ranges_for_claud")}
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr("new_tool_default")}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("tool")}</th>
                <th>{tr("scope")}</th>
                <th>{tr("default_model")}</th>
                <th>{tr("switching")}</th>
                <th>{tr("allowlist")}</th>
                <th>{tr("updated")}</th>
                <th className="cell-actions">{tr("actions")}</th>
              </tr>
            </thead>
            <tbody>
              {data.defaults.map((item) => (
                <tr key={item.id}>
                  <td>
                    <strong>{item.tool_key}</strong>
                  </td>
                  <td>
                    <Badge tone={item.scope_type === 'global' ? 'accent' : 'ok'}>{scopeLabel(item, tr)}</Badge>
                  </td>
                  <td>{item.default_model_client_name}</td>
                  <td>
                    <Badge tone={item.allow_switch ? 'ok' : ''}>
                      {item.allow_switch ? tr("allowed_2") : tr("fixed")}
                    </Badge>
                  </td>
                  <td>
                    {item.allowed_model_groups.length === 0
                      ? '—'
                      : item.allowed_model_groups.map((group) => group.client_model_name).join(', ')}
                  </td>
                  <td>{new Date(item.updated_at).toLocaleString('zh-CN')}</td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button className="btn sm" type="button" onClick={() => openEdit(item)}>
                        {tr("edit")}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteTarget(item)}>
                        {tr("delete")}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {modalOpen && data && (
        <Modal
          title={editId ? tr("edit_tool_default") : tr("new_tool_default")}
          sub={
            editId
              ? tr("scope_and_tool_identifier_cannot_be_changed")
              : 'global / user_group / user'
          }
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="tool-default-form" disabled={busy !== null}>
                {busy === 'save' ? tr("saving") : tr("save")}
              </button>
            </>
          }
        >
          <form id="tool-default-form" className="grid" onSubmit={(event) => void handleSave(event)} noValidate>
            {!editId && (
              <div className="grid cols-2">
                <div className="field">
                  <label htmlFor="tool-key">{tr("tool_identifier")}</label>
                  <select
                    id="tool-key"
                    className="select"
                    value={draft.tool_key}
                    onChange={(event) => setDraft({ ...draft, tool_key: event.target.value })}
                  >
                    {['claude_code', 'codex', 'gemini_cli', 'cursor', 'cline', 'cherry_studio'].map((tool) => (
                      <option key={tool} value={tool}>
                        {tool}
                      </option>
                    ))}
                  </select>
                  {errors.tool_key && <span className="field-error">{errors.tool_key}</span>}
                </div>
                <div className="field">
                  <label htmlFor="tool-scope">{tr("scope")}</label>
                  <select
                    id="tool-scope"
                    className="select"
                    value={draft.scope_type}
                    onChange={(event) =>
                      setDraft({
                        ...draft,
                        scope_type: event.target.value as Draft['scope_type'],
                        scope_id: '',
                      })
                    }
                  >
                    <option value="global">{tr("global")}</option>
                    <option value="user_group">{tr("user_group")}</option>
                    <option value="user">{tr("user")}</option>
                  </select>
                </div>
              </div>
            )}
            {!editId && draft.scope_type !== 'global' && (
              <div className="field">
                <label>{draft.scope_type === 'user_group' ? tr("user_group") : tr("user")}</label>
                <select
                  className="select"
                  value={draft.scope_id}
                  onChange={(event) => setDraft({ ...draft, scope_id: event.target.value })}
                >
                  <option value="">{tr("select")}</option>
                  {draft.scope_type === 'user_group'
                    ? data.userGroups.map((group: UserGroupSummary) => (
                        <option key={group.id} value={group.id}>
                          {group.name} · {group.member_count}{' '}
                          {tr(group.member_count === 1 ? 'member_count_unit_singular' : 'member_count_unit_plural')}
                        </option>
                      ))
                    : data.users.map((user: AdminUser) => (
                        <option key={user.id} value={user.id}>
                          {user.display_name || user.email} · {user.email}
                        </option>
                      ))}
                </select>
                {errors.scope_id && <span className="field-error">{errors.scope_id}</span>}
              </div>
            )}
            <div className="field">
              <label htmlFor="tool-default-group">{tr("default_model_group")}</label>
              <select
                id="tool-default-group"
                className="select"
                value={draft.default_model_group_id}
                onChange={(event) => setDraft({ ...draft, default_model_group_id: event.target.value })}
              >
                <option value="">{tr("select_an_active_model_group")}</option>
                {data.groups.map((group: ModelGroupSummary) => (
                  <option key={group.id} value={group.id}>
                    {group.client_model_name}
                  </option>
                ))}
              </select>
              {errors.default_model_group_id && (
                <span className="field-error">{errors.default_model_group_id}</span>
              )}
            </div>
            <div className="field">
              <label className="chk">
                <input
                  type="checkbox"
                  checked={draft.allow_switch}
                  onChange={(event) => setDraft({ ...draft, allow_switch: event.target.checked })}
                />
                {tr("allow_switching_within_the_allowlist")}
              </label>
            </div>
            {draft.allow_switch && (
              <div className="field">
                <label>{tr("switchable_model_groups_allowlist")}</label>
                <div className="toolbar" style={{ gap: 6 }}>
                  {data.groups.map((group: ModelGroupSummary) => (
                    <label key={group.id} className="chk">
                      <input
                        type="checkbox"
                        checked={draft.allowed_model_group_ids.includes(group.id)}
                        onChange={() => toggleWhitelist(group.id)}
                        disabled={group.id === draft.default_model_group_id}
                      />
                      {group.client_model_name}
                    </label>
                  ))}
                </div>
                {errors.allowed_model_group_ids && (
                  <span className="field-error">{errors.allowed_model_group_ids}</span>
                )}
              </div>
            )}
            {actionError && (
              <div className="form-error" role="alert">
                {actionError}
              </div>
            )}
          </form>
        </Modal>
      )}

      {deleteTarget && (
        <ConfirmDialog
          title={tr("delete_tool_default", { tool_key: deleteTarget.tool_key })}
          desc={
            <p>
              {tr("after_deletion_this_scope_will_have_no_tool_default_mode")}
            </p>
          }
          confirmText={tr("delete")}
          danger
          busy={busy === 'delete'}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => void handleDelete()}
        />
      )}
    </>
  )
}
