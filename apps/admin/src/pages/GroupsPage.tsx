import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import {
  createPairingCode,
  createUserGroup,
  deleteUserGroup,
  getUserGroup,
  listUserGroups,
  listUsers,
  updateUser,
  updateUserGroup,
} from '../api/accessApi'
import type { AdminUser, PairingCodeWithCode, UserGroupSummary } from '../api/access'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ConfirmDialog, Modal } from '../components/Modal'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { useI18n } from '../i18n/I18nProvider'

type Tab = 'users' | 'groups'

interface UserDraft {
  display_name: string
  status: 'active' | 'disabled'
}

interface GroupDraft {
  name: string
  description: string
  member_ids: string[]
}

const emptyGroup: GroupDraft = { name: '', description: '', member_ids: [] }

export function GroupsPage() {
  const { tr, locale } = useI18n()
  const [tab, setTab] = useState<Tab>('users')
  const { data, loading, error, reload } = useAsync(
    () =>
      Promise.all([listUsers(200), listUserGroups(200)]).then(([users, groups]) => ({
        users: users.items,
        groups: groups.items,
      })),
    [],
  )

  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [userDraft, setUserDraft] = useState<AdminUser | null>(null)
  const [userForm, setUserForm] = useState<UserDraft>({ display_name: '', status: 'active' })
  const [pairingCode, setPairingCode] = useState<PairingCodeWithCode | null>(null)
  const [groupModalOpen, setGroupModalOpen] = useState(false)
  const [groupEditId, setGroupEditId] = useState<string | null>(null)
  const [groupDraft, setGroupDraft] = useState<GroupDraft>(emptyGroup)
  const [groupErrors, setGroupErrors] = useState<Record<string, string>>({})
  const [deleteGroupTarget, setDeleteGroupTarget] = useState<UserGroupSummary | null>(null)

  function handleError(err: unknown, fallback: string) {
    setActionError(err instanceof ApiError ? err.detail : fallback)
  }

  function openEditUser(user: AdminUser) {
    setUserDraft(user)
    setUserForm({ display_name: user.display_name, status: user.status === 'disabled' ? 'disabled' : 'active' })
    setActionError(null)
  }

  async function handleSaveUser(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!userDraft) return
    setBusy('user-save')
    setActionError(null)
    try {
      await updateUser(userDraft.id, {
        display_name: userForm.display_name.trim(),
        status: userForm.status,
      })
      setUserDraft(null)
      reload()
    } catch (err) {
      handleError(err, tr("failed_to_update_employee"))
    } finally {
      setBusy(null)
    }
  }

  async function handlePairing(user: AdminUser) {
    setBusy(`pairing-${user.id}`)
    setActionError(null)
    try {
      const response = await createPairingCode({
        user_email: user.email,
        user_display_name: user.display_name || undefined,
        expires_in_seconds: 600,
      })
      setPairingCode(response.pairing_code)
    } catch (err) {
      handleError(err, tr("failed_to_generate_pairing_code"))
    } finally {
      setBusy(null)
    }
  }

  function openCreateGroup() {
    setGroupEditId(null)
    setGroupDraft(emptyGroup)
    setGroupErrors({})
    setActionError(null)
    setGroupModalOpen(true)
  }

  async function openEditGroup(group: UserGroupSummary) {
    setBusy(`group-load-${group.id}`)
    setActionError(null)
    try {
      const detail = await getUserGroup(group.id)
      setGroupEditId(group.id)
      setGroupDraft({
        name: detail.name,
        description: detail.description,
        member_ids: detail.members.map((member) => member.id),
      })
      setGroupErrors({})
      setGroupModalOpen(true)
    } catch (err) {
      handleError(err, tr("failed_to_load_user_group"))
    } finally {
      setBusy(null)
    }
  }

  async function handleSaveGroup(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('group-save')
    setGroupErrors({})
    setActionError(null)
    try {
      if (groupEditId) {
        await updateUserGroup(groupEditId, {
          name: groupDraft.name.trim(),
          description: groupDraft.description.trim(),
          member_ids: groupDraft.member_ids,
        })
      } else {
        await createUserGroup({
          name: groupDraft.name.trim(),
          description: groupDraft.description.trim(),
          member_ids: groupDraft.member_ids,
        })
      }
      setGroupModalOpen(false)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setGroupErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError(tr("failed_to_save_user_group"))
      }
    } finally {
      setBusy(null)
    }
  }

  async function handleDeleteGroup() {
    if (!deleteGroupTarget) return
    setBusy('group-delete')
    setActionError(null)
    try {
      await deleteUserGroup(deleteGroupTarget.id)
      setDeleteGroupTarget(null)
      reload()
    } catch (err) {
      handleError(err, tr("failed_to_delete_user_group"))
    } finally {
      setBusy(null)
    }
  }

  function toggleMember(userId: string) {
    setGroupDraft((current) => ({
      ...current,
      member_ids: current.member_ids.includes(userId)
        ? current.member_ids.filter((id) => id !== userId)
        : [...current.member_ids, userId],
    }))
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  const userCount = data?.users.length ?? 0
  const groupCount = data?.groups.length ?? 0

  return (
    <>
      <PageHead
        title={tr("users_groups")}
        desc={tr("manage_employee_status_and_group_access_groups_scope_mod")}
        actions={
          <>
            {refresh}
            {tab === 'groups' && (
              <button className="btn primary" type="button" onClick={openCreateGroup}>
                {tr("new_group")}
              </button>
            )}
          </>
        }
      />

      <div className="tabs">
        <button className={tab === 'users' ? 'active' : ''} type="button" onClick={() => setTab('users')}>
          {tr("employees", { userCount: userCount })}
        </button>
        <button className={tab === 'groups' ? 'active' : ''} type="button" onClick={() => setTab('groups')}>
          {tr("groups_2", { groupCount: groupCount })}
        </button>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr("loading_employees_and_groups")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : tab === 'users' ? (
        !data || data.users.length === 0 ? (
          <EmptyPanel
            title={tr("no_employees_yet")}
            desc={tr("employees_are_created_on_their_first_pairing_code_sign_i_2")}
          />
        ) : (
          <div className="table-wrap">
            <table className="table dense">
              <thead>
                <tr>
                  <th>{tr("employee")}</th>
                  <th>{tr("email")}</th>
                  <th>{tr("status")}</th>
                  <th className="cell-actions">{tr("actions")}</th>
                </tr>
              </thead>
              <tbody>
                {data.users.map((user) => {
                  return (
                    <tr key={user.id} className={user.status === 'disabled' ? 'is-off' : ''}>
                      <td>
                        <strong>{user.display_name || '—'}</strong>
                        <div className="hint">{user.id.slice(0, 8)}</div>
                      </td>
                      <td>{user.email}</td>
                      <td>
                        <Badge tone={user.status === 'active' ? 'ok' : 'bad'}>
                          {user.status === 'active' ? tr("active") : tr("disabled_2")}
                        </Badge>
                      </td>
                      <td className="cell-actions">
                        <span className="row-actions">
                          <button className="btn sm" type="button" onClick={() => openEditUser(user)}>
                            {tr("edit")}
                          </button>
                          <button
                            className="btn sm"
                            type="button"
                            disabled={busy !== null}
                            onClick={() => void handlePairing(user)}
                          >
                            {busy === `pairing-${user.id}` ? tr("generating") : tr("pairing_code")}
                          </button>
                        </span>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
            <div className="hint" style={{ padding: '8px 12px' }}>
              {tr("employees_are_created_on_their_first_pairing_code_sign_i")}
            </div>
          </div>
        )
      ) : !data || data.groups.length === 0 ? (
        <EmptyPanel
          title={tr("no_user_groups_yet")}
          desc={tr("groups_apply_model_publishing_tool_defaults_and_group_po")}
          action={
            <button className="btn primary" type="button" onClick={openCreateGroup}>
              {tr("new_group")}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("group_name")}</th>
                <th>{tr("description")}</th>
                <th>{tr("members_2")}</th>
                <th>{tr("updated")}</th>
                <th className="cell-actions">{tr("actions")}</th>
              </tr>
            </thead>
            <tbody>
              {data.groups.map((group) => (
                <tr key={group.id}>
                  <td>
                    <strong>{group.name}</strong>
                    <div className="hint">{group.id.slice(0, 8)}</div>
                  </td>
                  <td>{group.description || '—'}</td>
                  <td className="num">{group.member_count}</td>
                  <td>{new Date(group.updated_at).toLocaleString(locale)}</td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void openEditGroup(group)}
                      >
                        {busy === `group-load-${group.id}` ? tr("loading_2") : tr("edit")}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteGroupTarget(group)}>
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

      {userDraft && (
        <Modal
          title={tr("edit_employee")}
          sub={userDraft.email}
          onClose={() => setUserDraft(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setUserDraft(null)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="edit-user-form" disabled={busy !== null}>
                {busy === 'user-save' ? tr("saving") : tr("save")}
              </button>
            </>
          }
        >
          <form id="edit-user-form" className="grid" onSubmit={(event) => void handleSaveUser(event)} noValidate>
            <div className="field">
              <label htmlFor="user-display-name">{tr("display_name")}</label>
              <input
                id="user-display-name"
                className="input"
                value={userForm.display_name}
                onChange={(event) => setUserForm({ ...userForm, display_name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="user-status">{tr("status")}</label>
              <select
                id="user-status"
                className="select"
                value={userForm.status}
                onChange={(event) => setUserForm({ ...userForm, status: event.target.value as UserDraft['status'] })}
              >
                <option value="active">{tr("active")}</option>
                <option value="disabled">{tr("disabled")}</option>
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

      {pairingCode && (
        <Modal
          title={tr("pairing_code_generated")}
          sub={tr("shown_once_valid_for_10_minutes")}
          onClose={() => setPairingCode(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setPairingCode(null)}>
              {tr("close")}
            </button>
          }
        >
          <div className="field">
            <label>{tr("pairing_code")}</label>
            <div className="row">
              <input className="input" readOnly value={pairingCode.code} style={{ fontFamily: 'var(--mono)' }} />
              <button
                className="btn"
                type="button"
                onClick={() => void navigator.clipboard?.writeText(pairingCode.code)}
              >
                {tr("copy")}
              </button>
            </div>
            <span className="hint">
              {tr("employee_2")}
              {pairingCode.user.email} · {tr("valid_until")}{' '}
              {new Date(pairingCode.expires_at).toLocaleString(locale)}
            </span>
          </div>
          <p className="hint">
            {tr("enter_the_server_address_and_pairing_code_in_the_client")}
          </p>
        </Modal>
      )}

      {groupModalOpen && (
        <Modal
          title={groupEditId ? tr("edit_group") : tr("new_group")}
          onClose={() => setGroupModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setGroupModalOpen(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="group-form" disabled={busy !== null}>
                {busy === 'group-save' ? tr("saving") : tr("save")}
              </button>
            </>
          }
        >
          <form id="group-form" className="grid" onSubmit={(event) => void handleSaveGroup(event)} noValidate>
            <div className="field">
              <label htmlFor="group-name">{tr("group_name")}</label>
              <input
                id="group-name"
                className="input"
                placeholder={tr("e_g_support")}
                value={groupDraft.name}
                onChange={(event) => setGroupDraft({ ...groupDraft, name: event.target.value })}
              />
              {groupErrors.name && <span className="field-error">{groupErrors.name}</span>}
            </div>
            <div className="field">
              <label htmlFor="group-description">{tr("description")}</label>
              <input
                id="group-description"
                className="input"
                value={groupDraft.description}
                onChange={(event) => setGroupDraft({ ...groupDraft, description: event.target.value })}
              />
              {groupErrors.description && <span className="field-error">{groupErrors.description}</span>}
            </div>
            <div className="field">
              <label>
                {tr("members", { length: groupDraft.member_ids.length })}
              </label>
              <div className="table-wrap" style={{ maxHeight: 260 }}>
                <table className="table dense">
                  <thead>
                    <tr>
                      <th></th>
                      <th>{tr("employee")}</th>
                      <th>{tr("email")}</th>
                      <th>{tr("status")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data?.users.map((user) => (
                      <tr key={user.id}>
                        <td>
                          <input
                            type="checkbox"
                            checked={groupDraft.member_ids.includes(user.id)}
                            onChange={() => toggleMember(user.id)}
                          />
                        </td>
                        <td>{user.display_name || '—'}</td>
                        <td>{user.email}</td>
                        <td>{user.status}</td>
                      </tr>
                    ))}
                    {data && data.users.length === 0 && (
                      <tr>
                        <td colSpan={4} className="hint">
                          {tr("no_employees_yet_employees_are_created_after_their_first")}
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
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

      {deleteGroupTarget && (
        <ConfirmDialog
          title={tr("delete_group", { name: deleteGroupTarget.name })}
          desc={
            <p>
              {tr("deletes_the_group_and_its_memberships_existing_reference")}
            </p>
          }
          confirmText={tr("delete")}
          danger
          busy={busy === 'group-delete'}
          onCancel={() => setDeleteGroupTarget(null)}
          onConfirm={() => void handleDeleteGroup()}
        />
      )}
    </>
  )
}
