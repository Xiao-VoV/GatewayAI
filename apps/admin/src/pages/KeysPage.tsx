import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import { listKeysFiltered, resetKey, revokeKey, updateKey } from '../api/accessApi'
import type { KeyAdmin, ResetKeyResponse } from '../api/access'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ConfirmDialog, Modal } from '../components/Modal'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { useI18n } from '../i18n/I18nProvider'

interface KeyDraft {
  allowed_models: string
  rpm_limit: string
  expires_at: string
}

function toDatetimeLocal(iso: string | null) {
  if (!iso) return ''
  const date = new Date(iso)
  const offset = date.getTimezoneOffset()
  return new Date(date.getTime() - offset * 60_000).toISOString().slice(0, 16)
}

function fromDatetimeLocal(value: string) {
  return value ? new Date(value).toISOString() : null
}

function statusBadge(status: string, tr: (zh: string, en: string) => string) {
  if (status === 'active') return <Badge tone="ok">{tr('启用', 'Active')}</Badge>
  if (status === 'paused') return <Badge tone="warn">{tr('暂停', 'Paused')}</Badge>
  return <Badge tone="bad">{tr('已吊销', 'Revoked')}</Badge>
}

export function KeysPage() {
  const { tr, locale } = useI18n()
  const [statusFilter, setStatusFilter] = useState('')
  const { data, loading, error, reload } = useAsync(
    () => listKeysFiltered({ status: statusFilter || undefined, limit: 200 }),
    [statusFilter],
  )

  const [editKey, setEditKey] = useState<KeyAdmin | null>(null)
  const [draft, setDraft] = useState<KeyDraft>({ allowed_models: '', rpm_limit: '', expires_at: '' })
  const [errors, setErrors] = useState<Record<string, string>>({})
  const [actionError, setActionError] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [resetTarget, setResetTarget] = useState<KeyAdmin | null>(null)
  const [resetResult, setResetResult] = useState<ResetKeyResponse | null>(null)
  const [revokeTarget, setRevokeTarget] = useState<KeyAdmin | null>(null)

  function openEdit(key: KeyAdmin) {
    setEditKey(key)
    setDraft({
      allowed_models: key.allowed_models.join(', '),
      rpm_limit: key.rpm_limit === null ? '' : String(key.rpm_limit),
      expires_at: toDatetimeLocal(key.expires_at),
    })
    setErrors({})
    setActionError(null)
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!editKey) return
    setBusy('save')
    setErrors({})
    setActionError(null)
    try {
      await updateKey(editKey.id, {
        allowed_models: draft.allowed_models
          .split(',')
          .map((value) => value.trim())
          .filter(Boolean),
        rpm_limit: draft.rpm_limit.trim() ? Number(draft.rpm_limit) : null,
        expires_at: fromDatetimeLocal(draft.expires_at),
      })
      setEditKey(null)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError(tr('更新 Key 失败', 'Failed to update key'))
      }
    } finally {
      setBusy(null)
    }
  }

  async function handleReset() {
    if (!resetTarget) return
    setBusy('reset')
    setActionError(null)
    try {
      const response = await resetKey(resetTarget.id)
      setResetTarget(null)
      setResetResult(response)
      reload()
    } catch (err) {
      setActionError(err instanceof ApiError ? err.detail : tr('重置 Key 失败', 'Failed to reset key'))
    } finally {
      setBusy(null)
    }
  }

  async function handleRevoke() {
    if (!revokeTarget) return
    setBusy('revoke')
    setActionError(null)
    try {
      await revokeKey(revokeTarget.id)
      setRevokeTarget(null)
      reload()
    } catch (err) {
      setActionError(err instanceof ApiError ? err.detail : tr('吊销 Key 失败', 'Failed to revoke key'))
    } finally {
      setBusy(null)
    }
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr('虚拟 Key', 'Virtual keys')}
        desc={tr(
          '每设备一个 Key；白名单/RPM/有效期由 Key 收窄发布与预算。',
          'One key per device. The allowlist, RPM, and expiry further constrain access and budget at the key level.',
        )}
        actions={refresh}
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 160 }}>
            <label htmlFor="key-status">{tr('状态', 'Status')}</label>
            <select
              id="key-status"
              className="select"
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value)}
            >
              <option value="">{tr('全部', 'All')}</option>
              <option value="active">{tr('启用', 'Active')}</option>
              <option value="paused">{tr('暂停', 'Paused')}</option>
              <option value="revoked">{tr('已吊销', 'Revoked')}</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            {tr(
              'Key 由客户端配对时签发；管理台只改配置/重置/吊销，不创建新 Key。',
              'Keys are issued during client pairing. The console can edit, reset, or revoke them, but cannot create new keys.',
            )}
          </span>
        </div>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取虚拟 Key…', 'Loading virtual keys…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={statusFilter ? tr('没有匹配的 Key', 'No matching keys') : tr('还没有虚拟 Key', 'No virtual keys yet')}
          desc={tr(
            '员工通过配对码注册设备后，服务端会为该设备签发一个虚拟 Key。',
            'After an employee registers a device with a pairing code, the server issues a virtual key for that device.',
          )}
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('Key 前缀', 'Key prefix')}</th>
                <th>{tr('用户 / 设备', 'User / device')}</th>
                <th>{tr('状态', 'Status')}</th>
                <th>{tr('模型白名单', 'Model allowlist')}</th>
                <th>RPM</th>
                <th>{tr('过期', 'Expires')}</th>
                <th>{tr('最近使用', 'Last used')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
              </tr>
            </thead>
            <tbody>
              {data.items.map((key) => (
                <tr key={key.id} className={key.status === 'revoked' ? 'is-off' : ''}>
                  <td>
                    <code className="kbd">{key.key_prefix}</code>
                    <div className="hint">{key.id.slice(0, 8)}</div>
                  </td>
                  <td>
                    {key.user.display_name || key.user.email}
                    <div className="hint">{key.device.name}</div>
                  </td>
                  <td>{statusBadge(key.status, tr)}</td>
                  <td>{key.allowed_models.length === 0 ? tr('不限', 'Unlimited') : key.allowed_models.join(', ')}</td>
                  <td className="num">{key.rpm_limit ?? tr('不限', 'Unlimited')}</td>
                  <td>{key.expires_at ? new Date(key.expires_at).toLocaleString(locale) : tr('永不过期', 'Never')}</td>
                  <td>{key.last_used_at ? new Date(key.last_used_at).toLocaleString(locale) : '—'}</td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button className="btn sm" type="button" onClick={() => openEdit(key)}>
                        {tr('编辑', 'Edit')}
                      </button>
                      <button
                        className="btn sm"
                        type="button"
                        disabled={key.status === 'revoked'}
                        onClick={() => setResetTarget(key)}
                      >
                        {tr('重置', 'Reset')}
                      </button>
                      <button
                        className="btn sm danger"
                        type="button"
                        disabled={key.status === 'revoked'}
                        onClick={() => setRevokeTarget(key)}
                      >
                        {tr('吊销', 'Revoke')}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {tr(
              `共 ${data.total} 个 Key；同一用户多设备各自一个 Key，共用用户预算。`,
              `${data.total} keys total. Each device has its own key, and all share the user's budget.`,
            )}
          </div>
        </div>
      )}

      {editKey && (
        <Modal
          title={tr('编辑虚拟 Key', 'Edit virtual key')}
          sub={`${editKey.key_prefix} · ${editKey.user.email}`}
          onClose={() => setEditKey(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setEditKey(null)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="edit-key-form" disabled={busy !== null}>
                {busy === 'save' ? tr('保存中…', 'Saving…') : tr('保存', 'Save')}
              </button>
            </>
          }
        >
          <form id="edit-key-form" className="grid" onSubmit={(event) => void handleSave(event)} noValidate>
            <div className="field">
              <label htmlFor="key-allowed-models">
                {tr('模型白名单（逗号分隔，空=不限）', 'Model allowlist (comma-separated, empty = unlimited)')}
              </label>
              <input
                id="key-allowed-models"
                className="input"
                placeholder="model-group-a, model-group-b"
                value={draft.allowed_models}
                onChange={(event) => setDraft({ ...draft, allowed_models: event.target.value })}
              />
              {errors.allowed_models && <span className="field-error">{errors.allowed_models}</span>}
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="key-rpm">{tr('RPM 上限', 'RPM limit')}</label>
                <input
                  id="key-rpm"
                  className="input"
                  type="number"
                  min={1}
                  placeholder={tr('空=不限', 'Empty = unlimited')}
                  value={draft.rpm_limit}
                  onChange={(event) => setDraft({ ...draft, rpm_limit: event.target.value })}
                />
                {errors.rpm_limit && <span className="field-error">{errors.rpm_limit}</span>}
              </div>
              <div className="field">
                <label htmlFor="key-expires">{tr('过期时间（本地）', 'Expiry (local time)')}</label>
                <input
                  id="key-expires"
                  className="input"
                  type="datetime-local"
                  value={draft.expires_at}
                  onChange={(event) => setDraft({ ...draft, expires_at: event.target.value })}
                />
                {errors.expires_at && <span className="field-error">{errors.expires_at}</span>}
                <button className="btn sm" type="button" onClick={() => setDraft({ ...draft, expires_at: '' })}>
                  {tr('清除过期', 'Clear expiry')}
                </button>
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

      {resetTarget && (
        <ConfirmDialog
          title={tr(`重置 Key ${resetTarget.key_prefix}？`, `Reset key ${resetTarget.key_prefix}?`)}
          desc={
            <p>
              {tr(
                '旧 Key 立即失效；新明文 Key 只显示一次，请安全交给员工。',
                'The old key stops working immediately. The new plaintext key is shown only once, so deliver it to the employee securely.',
              )}
            </p>
          }
          confirmText={tr('重置', 'Reset')}
          danger
          busy={busy === 'reset'}
          onCancel={() => setResetTarget(null)}
          onConfirm={() => void handleReset()}
        />
      )}

      {revokeTarget && (
        <ConfirmDialog
          title={tr(`吊销 Key ${revokeTarget.key_prefix}？`, `Revoke key ${revokeTarget.key_prefix}?`)}
          desc={
            <p>
              {tr(
                '吊销后该设备请求立即 401；操作写入审计。重复吊销幂等。',
                'Requests from this device will immediately return 401. The action is recorded in the audit log and is idempotent.',
              )}
            </p>
          }
          confirmText={tr('吊销', 'Revoke')}
          danger
          busy={busy === 'revoke'}
          onCancel={() => setRevokeTarget(null)}
          onConfirm={() => void handleRevoke()}
        />
      )}

      {resetResult && (
        <Modal
          title={tr('新 Key 已生成', 'New key generated')}
          sub={tr('仅显示一次，请立即保存；旧 Key 已失效', 'Shown only once. Save it now; the old key is no longer valid.')}
          onClose={() => setResetResult(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setResetResult(null)}>
              {tr('我已保存', 'I saved it')}
            </button>
          }
        >
          <div className="field">
            <label>API Key</label>
            <div className="row">
              <input className="input" readOnly value={resetResult.api_key} style={{ fontFamily: 'var(--mono)' }} />
              <button className="btn" type="button" onClick={() => void navigator.clipboard?.writeText(resetResult.api_key)}>
                {tr('复制', 'Copy')}
              </button>
            </div>
            <span className="hint">
              {resetResult.key.user.email} · {resetResult.key.device.name}
            </span>
          </div>
        </Modal>
      )}
    </>
  )
}
