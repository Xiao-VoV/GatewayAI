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

function scopeLabel(defaultItem: ToolModelDefault) {
  if (defaultItem.scope_type === 'global') return '全局'
  return `${defaultItem.scope_type === 'user_group' ? '用户组' : '用户'} · ${defaultItem.scope_label ?? defaultItem.scope_id?.slice(0, 8) ?? '—'}`
}

export function ToolDefaultsPage() {
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
        setActionError('保存工具默认失败')
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
      setActionError(err instanceof ApiError ? err.detail : '删除工具默认失败')
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
      {loading ? '刷新中…' : '刷新'}
    </button>
  )

  return (
    <>
      <PageHead
        title="工具默认"
        desc="每工具默认模型与可切换白名单；user > user_group > global。"
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              新建工具默认
            </button>
          </>
        }
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 180 }}>
            <label htmlFor="tool-scope-filter">作用域</label>
            <select
              id="tool-scope-filter"
              className="select"
              value={scopeFilter}
              onChange={(event) => setScopeFilter(event.target.value)}
            >
              <option value="">全部</option>
              <option value="global">全局</option>
              <option value="user_group">用户组</option>
              <option value="user">用户</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            白名单只收窄发布；客户端实际默认需同时可见且可调用。
          </span>
        </div>
      </div>

      {loading && !data ? (
        <LoadingPanel title="正在读取工具默认…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.defaults.length === 0 ? (
        <EmptyPanel
          title="还没有工具默认"
          desc="为 Claude Code / Codex / Gemini CLI 等工具配置默认模型与可切换范围。"
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              新建工具默认
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>工具</th>
                <th>作用域</th>
                <th>默认模型</th>
                <th>允许切换</th>
                <th>白名单</th>
                <th>更新时间</th>
                <th className="cell-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {data.defaults.map((item) => (
                <tr key={item.id}>
                  <td>
                    <strong>{item.tool_key}</strong>
                  </td>
                  <td>
                    <Badge tone={item.scope_type === 'global' ? 'accent' : 'ok'}>{scopeLabel(item)}</Badge>
                  </td>
                  <td>{item.default_model_client_name}</td>
                  <td>
                    <Badge tone={item.allow_switch ? 'ok' : ''}>{item.allow_switch ? '允许' : '固定'}</Badge>
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
                        编辑
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteTarget(item)}>
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

      {modalOpen && data && (
        <Modal
          title={editId ? '编辑工具默认' : '新建工具默认'}
          sub={editId ? '作用域与工具标识不可修改' : 'global / user_group / user'}
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="tool-default-form" disabled={busy !== null}>
                {busy === 'save' ? '保存中…' : '保存'}
              </button>
            </>
          }
        >
          <form id="tool-default-form" className="grid" onSubmit={(event) => void handleSave(event)} noValidate>
            {!editId && (
              <div className="grid cols-2">
                <div className="field">
                  <label htmlFor="tool-key">工具标识</label>
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
                  <label htmlFor="tool-scope">作用域</label>
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
                    <option value="global">全局</option>
                    <option value="user_group">用户组</option>
                    <option value="user">用户</option>
                  </select>
                </div>
              </div>
            )}
            {!editId && draft.scope_type !== 'global' && (
              <div className="field">
                <label>{draft.scope_type === 'user_group' ? '用户组' : '用户'}</label>
                <select
                  className="select"
                  value={draft.scope_id}
                  onChange={(event) => setDraft({ ...draft, scope_id: event.target.value })}
                >
                  <option value="">请选择</option>
                  {draft.scope_type === 'user_group'
                    ? data.userGroups.map((group: UserGroupSummary) => (
                        <option key={group.id} value={group.id}>
                          {group.name} · {group.member_count} 人
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
              <label htmlFor="tool-default-group">默认模型组</label>
              <select
                id="tool-default-group"
                className="select"
                value={draft.default_model_group_id}
                onChange={(event) => setDraft({ ...draft, default_model_group_id: event.target.value })}
              >
                <option value="">请选择 active 模型组</option>
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
                允许在白名单内切换
              </label>
            </div>
            {draft.allow_switch && (
              <div className="field">
                <label>可切换模型组（白名单）</label>
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
          title={`删除工具默认 ${deleteTarget.tool_key}？`}
          desc={<p>删除后该作用域不再有工具默认；不会删除模型组。</p>}
          confirmText="删除"
          danger
          busy={busy === 'delete'}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => void handleDelete()}
        />
      )}
    </>
  )
}
