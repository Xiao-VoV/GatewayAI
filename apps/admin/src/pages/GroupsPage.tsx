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
      handleError(err, tr('更新员工失败', 'Failed to update employee'))
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
      handleError(err, tr('生成配对码失败', 'Failed to generate pairing code'))
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
      handleError(err, tr('读取用户组失败', 'Failed to load user group'))
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
        setActionError(tr('保存用户组失败', 'Failed to save user group'))
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
      handleError(err, tr('删除用户组失败', 'Failed to delete user group'))
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
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  const userCount = data?.users.length ?? 0
  const groupCount = data?.groups.length ?? 0

  return (
    <>
      <PageHead
        title={tr('用户与用户组', 'Users & groups')}
        desc={tr(
          '员工状态与组授权；组用于发布/工具默认的作用域。',
          'Manage employee status and group access. Groups scope model publishing and tool defaults.',
        )}
        actions={
          <>
            {refresh}
            {tab === 'groups' && (
              <button className="btn primary" type="button" onClick={openCreateGroup}>
                {tr('新建用户组', 'New group')}
              </button>
            )}
          </>
        }
      />

      <div className="tabs">
        <button className={tab === 'users' ? 'active' : ''} type="button" onClick={() => setTab('users')}>
          {tr(`员工（${userCount}）`, `Employees (${userCount})`)}
        </button>
        <button className={tab === 'groups' ? 'active' : ''} type="button" onClick={() => setTab('groups')}>
          {tr(`用户组（${groupCount}）`, `Groups (${groupCount})`)}
        </button>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取员工与用户组…', 'Loading employees and groups…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : tab === 'users' ? (
        !data || data.users.length === 0 ? (
          <EmptyPanel
            title={tr('还没有员工', 'No employees yet')}
            desc={tr(
              '员工首次通过配对码登录时自动创建；可先创建配对码发给员工。',
              'Employees are created on their first pairing-code sign-in. Generate a pairing code and share it with them to get started.',
            )}
          />
        ) : (
          <div className="table-wrap">
            <table className="table dense">
              <thead>
                <tr>
                  <th>{tr('员工', 'Employee')}</th>
                  <th>{tr('邮箱', 'Email')}</th>
                  <th>{tr('状态', 'Status')}</th>
                  <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                          {user.status === 'active' ? tr('正常', 'Active') : tr('已禁用', 'Disabled')}
                        </Badge>
                      </td>
                      <td className="cell-actions">
                        <span className="row-actions">
                          <button className="btn sm" type="button" onClick={() => openEditUser(user)}>
                            {tr('编辑', 'Edit')}
                          </button>
                          <button
                            className="btn sm"
                            type="button"
                            disabled={busy !== null}
                            onClick={() => void handlePairing(user)}
                          >
                            {busy === `pairing-${user.id}` ? tr('生成中…', 'Generating…') : tr('配对码', 'Pairing code')}
                          </button>
                        </span>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
            <div className="hint" style={{ padding: '8px 12px' }}>
              {tr(
                '员工由配对码首次登录时创建；禁用后其所有 Key 请求 403。',
                'Employees are created on their first pairing-code sign-in. Disabled employees receive 403 on all key requests.',
              )}
            </div>
          </div>
        )
      ) : !data || data.groups.length === 0 ? (
        <EmptyPanel
          title={tr('还没有用户组', 'No user groups yet')}
          desc={tr(
            '用户组用于批量发布模型、工具默认和组策略。',
            'Groups apply model publishing, tool defaults, and group policies to many employees at once.',
          )}
          action={
            <button className="btn primary" type="button" onClick={openCreateGroup}>
              {tr('新建用户组', 'New group')}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('组名', 'Group name')}</th>
                <th>{tr('描述', 'Description')}</th>
                <th>{tr('成员', 'Members')}</th>
                <th>{tr('更新时间', 'Updated')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                        {busy === `group-load-${group.id}` ? tr('读取中…', 'Loading…') : tr('编辑', 'Edit')}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteGroupTarget(group)}>
                        {tr('删除', 'Delete')}
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
          title={tr('编辑员工', 'Edit employee')}
          sub={userDraft.email}
          onClose={() => setUserDraft(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setUserDraft(null)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="edit-user-form" disabled={busy !== null}>
                {busy === 'user-save' ? tr('保存中…', 'Saving…') : tr('保存', 'Save')}
              </button>
            </>
          }
        >
          <form id="edit-user-form" className="grid" onSubmit={(event) => void handleSaveUser(event)} noValidate>
            <div className="field">
              <label htmlFor="user-display-name">{tr('显示名', 'Display name')}</label>
              <input
                id="user-display-name"
                className="input"
                value={userForm.display_name}
                onChange={(event) => setUserForm({ ...userForm, display_name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="user-status">{tr('状态', 'Status')}</label>
              <select
                id="user-status"
                className="select"
                value={userForm.status}
                onChange={(event) => setUserForm({ ...userForm, status: event.target.value as UserDraft['status'] })}
              >
                <option value="active">{tr('正常', 'Active')}</option>
                <option value="disabled">{tr('禁用', 'Disabled')}</option>
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
          title={tr('配对码已生成', 'Pairing code generated')}
          sub={tr('仅显示一次；10 分钟内有效', 'Shown once; valid for 10 minutes')}
          onClose={() => setPairingCode(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setPairingCode(null)}>
              {tr('关闭', 'Close')}
            </button>
          }
        >
          <div className="field">
            <label>{tr('配对码', 'Pairing code')}</label>
            <div className="row">
              <input className="input" readOnly value={pairingCode.code} style={{ fontFamily: 'var(--mono)' }} />
              <button
                className="btn"
                type="button"
                onClick={() => void navigator.clipboard?.writeText(pairingCode.code)}
              >
                {tr('复制', 'Copy')}
              </button>
            </div>
            <span className="hint">
              {tr('员工：', 'Employee: ')}
              {pairingCode.user.email} · {tr('有效期至', 'Valid until')}{' '}
              {new Date(pairingCode.expires_at).toLocaleString(locale)}
            </span>
          </div>
          <p className="hint">
            {tr(
              '在客户端输入服务器地址 + 配对码完成设备注册与 Key 下发。',
              'Enter the server address and pairing code in the client to register the device and issue a key.',
            )}
          </p>
        </Modal>
      )}

      {groupModalOpen && (
        <Modal
          title={groupEditId ? tr('编辑用户组', 'Edit group') : tr('新建用户组', 'New group')}
          onClose={() => setGroupModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setGroupModalOpen(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="group-form" disabled={busy !== null}>
                {busy === 'group-save' ? tr('保存中…', 'Saving…') : tr('保存', 'Save')}
              </button>
            </>
          }
        >
          <form id="group-form" className="grid" onSubmit={(event) => void handleSaveGroup(event)} noValidate>
            <div className="field">
              <label htmlFor="group-name">{tr('组名', 'Group name')}</label>
              <input
                id="group-name"
                className="input"
                placeholder={tr('例如 客服组', 'e.g. Support')}
                value={groupDraft.name}
                onChange={(event) => setGroupDraft({ ...groupDraft, name: event.target.value })}
              />
              {groupErrors.name && <span className="field-error">{groupErrors.name}</span>}
            </div>
            <div className="field">
              <label htmlFor="group-description">{tr('描述', 'Description')}</label>
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
                {tr(
                  `成员（${groupDraft.member_ids.length}）`,
                  `Members (${groupDraft.member_ids.length})`,
                )}
              </label>
              <div className="table-wrap" style={{ maxHeight: 260 }}>
                <table className="table dense">
                  <thead>
                    <tr>
                      <th></th>
                      <th>{tr('员工', 'Employee')}</th>
                      <th>{tr('邮箱', 'Email')}</th>
                      <th>{tr('状态', 'Status')}</th>
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
                          {tr(
                            '还没有员工；员工通过配对码首次登录后创建。',
                            'No employees yet. Employees are created after their first pairing-code sign-in.',
                          )}
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
          title={tr(`删除用户组 ${deleteGroupTarget.name}？`, `Delete group ${deleteGroupTarget.name}?`)}
          desc={
            <p>
              {tr(
                '删除组及其成员关系；发布对象/工具默认中的历史组引用将失效。操作写入审计。',
                'Deletes the group and its memberships. Existing references in publishing targets and tool defaults will stop resolving. This action is recorded in the audit log.',
              )}
            </p>
          }
          confirmText={tr('删除', 'Delete')}
          danger
          busy={busy === 'group-delete'}
          onCancel={() => setDeleteGroupTarget(null)}
          onConfirm={() => void handleDeleteGroup()}
        />
      )}
    </>
  )
}
