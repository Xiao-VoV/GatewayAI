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
      handleError(err, '更新员工失败')
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
      handleError(err, '生成配对码失败')
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
      handleError(err, '读取用户组失败')
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
        setActionError('保存用户组失败')
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
      handleError(err, '删除用户组失败')
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
      {loading ? '刷新中…' : '刷新'}
    </button>
  )

  return (
    <>
      <PageHead
        title="用户与用户组"
        desc="员工状态与组授权；组用于发布/工具默认的作用域。"
        actions={
          <>
            {refresh}
            {tab === 'groups' && (
              <button className="btn primary" type="button" onClick={openCreateGroup}>
                新建用户组
              </button>
            )}
          </>
        }
      />

      <div className="tabs">
        <button className={tab === 'users' ? 'active' : ''} type="button" onClick={() => setTab('users')}>
          员工（{data?.users.length ?? 0}）
        </button>
        <button className={tab === 'groups' ? 'active' : ''} type="button" onClick={() => setTab('groups')}>
          用户组（{data?.groups.length ?? 0}）
        </button>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title="正在读取员工与用户组…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : tab === 'users' ? (
        !data || data.users.length === 0 ? (
          <EmptyPanel
            title="还没有员工"
            desc="员工首次通过配对码登录时自动创建；可先创建配对码发给员工。"
          />
        ) : (
          <div className="table-wrap">
            <table className="table dense">
              <thead>
                <tr>
                  <th>员工</th>
                  <th>邮箱</th>
                  <th>状态</th>
                  <th className="cell-actions">操作</th>
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
                          {user.status === 'active' ? '正常' : '已禁用'}
                        </Badge>
                      </td>
                      <td className="cell-actions">
                        <span className="row-actions">
                          <button className="btn sm" type="button" onClick={() => openEditUser(user)}>
                            编辑
                          </button>
                          <button
                            className="btn sm"
                            type="button"
                            disabled={busy !== null}
                            onClick={() => void handlePairing(user)}
                          >
                            {busy === `pairing-${user.id}` ? '生成中…' : '配对码'}
                          </button>
                        </span>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
            <div className="hint" style={{ padding: '8px 12px' }}>
              员工由配对码首次登录时创建；禁用后其所有 Key 请求 403。
            </div>
          </div>
        )
      ) : !data || data.groups.length === 0 ? (
        <EmptyPanel
          title="还没有用户组"
          desc="用户组用于批量发布模型、工具默认和组策略。"
          action={
            <button className="btn primary" type="button" onClick={openCreateGroup}>
              新建用户组
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>组名</th>
                <th>描述</th>
                <th>成员</th>
                <th>更新时间</th>
                <th className="cell-actions">操作</th>
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
                  <td>{new Date(group.updated_at).toLocaleString('zh-CN')}</td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button
                        className="btn sm"
                        type="button"
                        disabled={busy !== null}
                        onClick={() => void openEditGroup(group)}
                      >
                        {busy === `group-load-${group.id}` ? '读取中…' : '编辑'}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteGroupTarget(group)}>
                        删除
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
          title="编辑员工"
          sub={userDraft.email}
          onClose={() => setUserDraft(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setUserDraft(null)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="edit-user-form" disabled={busy !== null}>
                {busy === 'user-save' ? '保存中…' : '保存'}
              </button>
            </>
          }
        >
          <form id="edit-user-form" className="grid" onSubmit={(event) => void handleSaveUser(event)} noValidate>
            <div className="field">
              <label htmlFor="user-display-name">显示名</label>
              <input
                id="user-display-name"
                className="input"
                value={userForm.display_name}
                onChange={(event) => setUserForm({ ...userForm, display_name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="user-status">状态</label>
              <select
                id="user-status"
                className="select"
                value={userForm.status}
                onChange={(event) => setUserForm({ ...userForm, status: event.target.value as UserDraft['status'] })}
              >
                <option value="active">正常</option>
                <option value="disabled">禁用</option>
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
          title="配对码已生成"
          sub="仅显示一次；10 分钟内有效"
          onClose={() => setPairingCode(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setPairingCode(null)}>
              关闭
            </button>
          }
        >
          <div className="field">
            <label>配对码</label>
            <div className="row">
              <input className="input" readOnly value={pairingCode.code} style={{ fontFamily: 'var(--mono)' }} />
              <button
                className="btn"
                type="button"
                onClick={() => void navigator.clipboard?.writeText(pairingCode.code)}
              >
                复制
              </button>
            </div>
            <span className="hint">
              员工：{pairingCode.user.email} · 有效期至{' '}
              {new Date(pairingCode.expires_at).toLocaleString('zh-CN')}
            </span>
          </div>
          <p className="hint">在客户端输入服务器地址 + 配对码完成设备注册与 Key 下发。</p>
        </Modal>
      )}

      {groupModalOpen && (
        <Modal
          title={groupEditId ? '编辑用户组' : '新建用户组'}
          onClose={() => setGroupModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setGroupModalOpen(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="group-form" disabled={busy !== null}>
                {busy === 'group-save' ? '保存中…' : '保存'}
              </button>
            </>
          }
        >
          <form id="group-form" className="grid" onSubmit={(event) => void handleSaveGroup(event)} noValidate>
            <div className="field">
              <label htmlFor="group-name">组名</label>
              <input
                id="group-name"
                className="input"
                placeholder="例如 客服组"
                value={groupDraft.name}
                onChange={(event) => setGroupDraft({ ...groupDraft, name: event.target.value })}
              />
              {groupErrors.name && <span className="field-error">{groupErrors.name}</span>}
            </div>
            <div className="field">
              <label htmlFor="group-description">描述</label>
              <input
                id="group-description"
                className="input"
                value={groupDraft.description}
                onChange={(event) => setGroupDraft({ ...groupDraft, description: event.target.value })}
              />
              {groupErrors.description && <span className="field-error">{groupErrors.description}</span>}
            </div>
            <div className="field">
              <label>成员（{groupDraft.member_ids.length}）</label>
              <div className="table-wrap" style={{ maxHeight: 260 }}>
                <table className="table dense">
                  <thead>
                    <tr>
                      <th></th>
                      <th>员工</th>
                      <th>邮箱</th>
                      <th>状态</th>
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
                          还没有员工；员工通过配对码首次登录后创建。
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
          title={`删除用户组 ${deleteGroupTarget.name}？`}
          desc={<p>删除组及其成员关系；发布对象/工具默认中的历史组引用将失效。操作写入审计。</p>}
          confirmText="删除"
          danger
          busy={busy === 'group-delete'}
          onCancel={() => setDeleteGroupTarget(null)}
          onConfirm={() => void handleDeleteGroup()}
        />
      )}
    </>
  )
}
