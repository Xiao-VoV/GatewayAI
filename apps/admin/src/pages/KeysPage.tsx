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
import type { Translate } from '../i18n/translate'

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

function statusBadge(status: string, tr: Translate) {
  if (status === 'active') return <Badge tone="ok">{tr("active_2")}</Badge>
  if (status === 'paused') return <Badge tone="warn">{tr("paused")}</Badge>
  return <Badge tone="bad">{tr("revoked_2")}</Badge>
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
        setActionError(tr("failed_to_update_key"))
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
      setActionError(err instanceof ApiError ? err.detail : tr("failed_to_reset_key"))
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
      setActionError(err instanceof ApiError ? err.detail : tr("failed_to_revoke_key"))
    } finally {
      setBusy(null)
    }
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr("virtual_keys")}
        desc={tr("one_key_per_device_the_allowlist_rpm_and_expiry_further")}
        actions={refresh}
      />

      <div className="card">
        <div className="toolbar">
          <div className="field" style={{ width: 160 }}>
            <label htmlFor="key-status">{tr("status")}</label>
            <select
              id="key-status"
              className="select"
              value={statusFilter}
              onChange={(event) => setStatusFilter(event.target.value)}
            >
              <option value="">{tr("all")}</option>
              <option value="active">{tr("active_2")}</option>
              <option value="paused">{tr("paused")}</option>
              <option value="revoked">{tr("revoked_2")}</option>
            </select>
          </div>
          <span className="hint" style={{ marginTop: 18 }}>
            {tr("keys_are_issued_during_client_pairing_the_console_can_ed")}
          </span>
        </div>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr("loading_virtual_keys")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={statusFilter ? tr("no_matching_keys") : tr("no_virtual_keys_yet")}
          desc={tr("after_an_employee_registers_a_device_with_a_pairing_code")}
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("key_prefix")}</th>
                <th>{tr("user_device")}</th>
                <th>{tr("status")}</th>
                <th>{tr("model_allowlist")}</th>
                <th>RPM</th>
                <th>{tr("expires")}</th>
                <th>{tr("last_used")}</th>
                <th className="cell-actions">{tr("actions")}</th>
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
                  <td>{key.allowed_models.length === 0 ? tr("unlimited") : key.allowed_models.join(', ')}</td>
                  <td className="num">{key.rpm_limit ?? tr("unlimited")}</td>
                  <td>{key.expires_at ? new Date(key.expires_at).toLocaleString(locale) : tr("never")}</td>
                  <td>{key.last_used_at ? new Date(key.last_used_at).toLocaleString(locale) : '—'}</td>
                  <td className="cell-actions">
                    <span className="row-actions">
                      <button className="btn sm" type="button" onClick={() => openEdit(key)}>
                        {tr("edit")}
                      </button>
                      <button
                        className="btn sm"
                        type="button"
                        disabled={key.status === 'revoked'}
                        onClick={() => setResetTarget(key)}
                      >
                        {tr("reset")}
                      </button>
                      <button
                        className="btn sm danger"
                        type="button"
                        disabled={key.status === 'revoked'}
                        onClick={() => setRevokeTarget(key)}
                      >
                        {tr("revoke_2")}
                      </button>
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {tr("keys_total_each_device_has_its_own_key_and_all_share_the", { total: data.total })}
          </div>
        </div>
      )}

      {editKey && (
        <Modal
          title={tr("edit_virtual_key")}
          sub={`${editKey.key_prefix} · ${editKey.user.email}`}
          onClose={() => setEditKey(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setEditKey(null)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="edit-key-form" disabled={busy !== null}>
                {busy === 'save' ? tr("saving") : tr("save")}
              </button>
            </>
          }
        >
          <form id="edit-key-form" className="grid" onSubmit={(event) => void handleSave(event)} noValidate>
            <div className="field">
              <label htmlFor="key-allowed-models">
                {tr("model_allowlist_comma_separated_empty_unlimited")}
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
                <label htmlFor="key-rpm">{tr("rpm_limit")}</label>
                <input
                  id="key-rpm"
                  className="input"
                  type="number"
                  min={1}
                  placeholder={tr("empty_unlimited")}
                  value={draft.rpm_limit}
                  onChange={(event) => setDraft({ ...draft, rpm_limit: event.target.value })}
                />
                {errors.rpm_limit && <span className="field-error">{errors.rpm_limit}</span>}
              </div>
              <div className="field">
                <label htmlFor="key-expires">{tr("expiry_local_time")}</label>
                <input
                  id="key-expires"
                  className="input"
                  type="datetime-local"
                  value={draft.expires_at}
                  onChange={(event) => setDraft({ ...draft, expires_at: event.target.value })}
                />
                {errors.expires_at && <span className="field-error">{errors.expires_at}</span>}
                <button className="btn sm" type="button" onClick={() => setDraft({ ...draft, expires_at: '' })}>
                  {tr("clear_expiry")}
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
          title={tr("reset_key", { key_prefix: resetTarget.key_prefix })}
          desc={
            <p>
              {tr("the_old_key_stops_working_immediately_the_new_plaintext")}
            </p>
          }
          confirmText={tr("reset")}
          danger
          busy={busy === 'reset'}
          onCancel={() => setResetTarget(null)}
          onConfirm={() => void handleReset()}
        />
      )}

      {revokeTarget && (
        <ConfirmDialog
          title={tr("revoke_key", { key_prefix: revokeTarget.key_prefix })}
          desc={
            <p>
              {tr("requests_from_this_device_will_immediately_return_401_th")}
            </p>
          }
          confirmText={tr("revoke_2")}
          danger
          busy={busy === 'revoke'}
          onCancel={() => setRevokeTarget(null)}
          onConfirm={() => void handleRevoke()}
        />
      )}

      {resetResult && (
        <Modal
          title={tr("new_key_generated")}
          sub={tr("shown_only_once_save_it_now_the_old_key_is_no_longer_val")}
          onClose={() => setResetResult(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setResetResult(null)}>
              {tr("i_saved_it")}
            </button>
          }
        >
          <div className="field">
            <label>API Key</label>
            <div className="row">
              <input className="input" readOnly value={resetResult.api_key} style={{ fontFamily: 'var(--mono)' }} />
              <button className="btn" type="button" onClick={() => void navigator.clipboard?.writeText(resetResult.api_key)}>
                {tr("copy")}
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
