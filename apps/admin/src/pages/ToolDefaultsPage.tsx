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

function scopeLabel(defaultItem: ToolModelDefault, tr: (zh: string, en: string) => string) {
  if (defaultItem.scope_type === 'global') return tr('全局', 'Global')
  return `${defaultItem.scope_type === 'user_group' ? tr('用户组', 'User group') : tr('用户', 'User')} · ${defaultItem.scope_label ?? defaultItem.scope_id?.slice(0, 8) ?? '—'}`
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
        setActionError(tr('保存工具默认失败', 'Failed to save tool default'))
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
      setActionError(err instanceof ApiError ? err.detail : tr('删除工具默认失败', 'Failed to delete tool default'))
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
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr('工具默认', 'Tool defaults')}
        desc={tr(
          '每工具默认模型与可切换白名单；user > user_group > global。',
          'Default model per tool and switchable allowlist; user > user_group > global.',
        )}
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr('新建工具默认', 'New tool default')}
            </button>
          </>
        }
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 180 }}>
            <label htmlFor="tool-scope-filter">{tr('作用域', 'Scope')}</label>
            <select
              id="tool-scope-filter"
              className="select"
              value={scopeFilter}
              onChange={(event) => setScopeFilter(event.target.value)}
            >
              <option value="">{tr('全部', 'All')}</option>
              <option value="global">{tr('全局', 'Global')}</option>
              <option value="user_group">{tr('用户组', 'User group')}</option>
              <option value="user">{tr('用户', 'User')}</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            {tr(
              '白名单只收窄发布；客户端实际默认需同时可见且可调用。',
              'The allowlist only narrows publishing; the effective client default must be visible and callable.',
            )}
          </span>
        </div>
      </div>

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取工具默认…', 'Loading tool defaults…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.defaults.length === 0 ? (
        <EmptyPanel
          title={tr('还没有工具默认', 'No tool defaults yet')}
          desc={tr(
            '为 Claude Code / Codex / Gemini CLI 等工具配置默认模型与可切换范围。',
            'Configure default models and switchable ranges for Claude Code, Codex, Gemini CLI, and more.',
          )}
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr('新建工具默认', 'New tool default')}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('工具', 'Tool')}</th>
                <th>{tr('作用域', 'Scope')}</th>
                <th>{tr('默认模型', 'Default model')}</th>
                <th>{tr('允许切换', 'Switching')}</th>
                <th>{tr('白名单', 'Allowlist')}</th>
                <th>{tr('更新时间', 'Updated')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                      {item.allow_switch ? tr('允许', 'Allowed') : tr('固定', 'Fixed')}
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
                        {tr('编辑', 'Edit')}
                      </button>
                      <button className="btn sm danger" type="button" onClick={() => setDeleteTarget(item)}>
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

      {modalOpen && data && (
        <Modal
          title={editId ? tr('编辑工具默认', 'Edit tool default') : tr('新建工具默认', 'New tool default')}
          sub={
            editId
              ? tr('作用域与工具标识不可修改', 'Scope and tool identifier cannot be changed')
              : 'global / user_group / user'
          }
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="tool-default-form" disabled={busy !== null}>
                {busy === 'save' ? tr('保存中…', 'Saving…') : tr('保存', 'Save')}
              </button>
            </>
          }
        >
          <form id="tool-default-form" className="grid" onSubmit={(event) => void handleSave(event)} noValidate>
            {!editId && (
              <div className="grid cols-2">
                <div className="field">
                  <label htmlFor="tool-key">{tr('工具标识', 'Tool identifier')}</label>
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
                  <label htmlFor="tool-scope">{tr('作用域', 'Scope')}</label>
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
                    <option value="global">{tr('全局', 'Global')}</option>
                    <option value="user_group">{tr('用户组', 'User group')}</option>
                    <option value="user">{tr('用户', 'User')}</option>
                  </select>
                </div>
              </div>
            )}
            {!editId && draft.scope_type !== 'global' && (
              <div className="field">
                <label>{draft.scope_type === 'user_group' ? tr('用户组', 'User group') : tr('用户', 'User')}</label>
                <select
                  className="select"
                  value={draft.scope_id}
                  onChange={(event) => setDraft({ ...draft, scope_id: event.target.value })}
                >
                  <option value="">{tr('请选择', 'Select…')}</option>
                  {draft.scope_type === 'user_group'
                    ? data.userGroups.map((group: UserGroupSummary) => (
                        <option key={group.id} value={group.id}>
                          {group.name} · {group.member_count}{' '}
                          {tr('人', group.member_count === 1 ? 'member' : 'members')}
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
              <label htmlFor="tool-default-group">{tr('默认模型组', 'Default model group')}</label>
              <select
                id="tool-default-group"
                className="select"
                value={draft.default_model_group_id}
                onChange={(event) => setDraft({ ...draft, default_model_group_id: event.target.value })}
              >
                <option value="">{tr('请选择 active 模型组', 'Select an active model group')}</option>
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
                {tr('允许在白名单内切换', 'Allow switching within the allowlist')}
              </label>
            </div>
            {draft.allow_switch && (
              <div className="field">
                <label>{tr('可切换模型组（白名单）', 'Switchable model groups (allowlist)')}</label>
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
          title={tr(`删除工具默认 ${deleteTarget.tool_key}？`, `Delete tool default ${deleteTarget.tool_key}?`)}
          desc={
            <p>
              {tr(
                '删除后该作用域不再有工具默认；不会删除模型组。',
                'After deletion, this scope will have no tool default; model groups are not deleted.',
              )}
            </p>
          }
          confirmText={tr('删除', 'Delete')}
          danger
          busy={busy === 'delete'}
          onCancel={() => setDeleteTarget(null)}
          onConfirm={() => void handleDelete()}
        />
      )}
    </>
  )
}
