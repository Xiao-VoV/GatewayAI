import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import {
  createPairingCode,
  invalidatePairingCode,
  listDevicesFiltered,
  listPairingCodes,
  unbindDevice,
  updateDevice,
} from '../api/accessApi'
import type { DeviceAdmin, PairingCode, PairingCodeWithCode } from '../api/access'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ConfirmDialog, Modal } from '../components/Modal'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { useI18n } from '../i18n/I18nProvider'
import type { Translate } from '../i18n/translate'

type Tab = 'devices' | 'pairing'

interface DeviceDraft {
  name: string
  status: 'active' | 'disabled'
}

interface PairingDraft {
  user_email: string
  user_display_name: string
  ttl: string
}

const emptyPairing: PairingDraft = { user_email: '', user_display_name: '', ttl: '600' }

function pairingStatusBadge(status: string, tr: Translate) {
  if (status === 'active') return <Badge tone="ok">{tr("unused")}</Badge>
  if (status === 'used') return <Badge>{tr("used")}</Badge>
  if (status === 'expired') return <Badge tone="warn">{tr("expired")}</Badge>
  return <Badge tone="warn">{tr("revoked")}</Badge>
}

export function DevicesPage() {
  const { tr } = useI18n()
  const [tab, setTab] = useState<Tab>('devices')
  const [statusFilter, setStatusFilter] = useState('')
  const { data, loading, error, reload } = useAsync(
    () =>
      Promise.all([
        listDevicesFiltered({ status: statusFilter || undefined, limit: 200 }),
        listPairingCodes(100),
      ]).then(([devices, codes]) => ({ devices: devices.items, codes: codes.items })),
    [statusFilter],
  )

  const [busy, setBusy] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [deviceEdit, setDeviceEdit] = useState<DeviceAdmin | null>(null)
  const [deviceDraft, setDeviceDraft] = useState<DeviceDraft>({ name: '', status: 'active' })
  const [unbindTarget, setUnbindTarget] = useState<DeviceAdmin | null>(null)
  const [pairingOpen, setPairingOpen] = useState(false)
  const [pairingDraft, setPairingDraft] = useState<PairingDraft>(emptyPairing)
  const [pairingErrors, setPairingErrors] = useState<Record<string, string>>({})
  const [createdCode, setCreatedCode] = useState<PairingCodeWithCode | null>(null)
  const [invalidateTarget, setInvalidateTarget] = useState<PairingCode | null>(null)

  function handleError(err: unknown, fallback: string) {
    setActionError(err instanceof ApiError ? err.detail : fallback)
  }

  function openEditDevice(device: DeviceAdmin) {
    setDeviceEdit(device)
    setDeviceDraft({ name: device.name, status: device.status === 'disabled' ? 'disabled' : 'active' })
    setActionError(null)
  }

  async function handleSaveDevice(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!deviceEdit) return
    setBusy('device-save')
    setActionError(null)
    try {
      await updateDevice(deviceEdit.id, { name: deviceDraft.name.trim(), status: deviceDraft.status })
      setDeviceEdit(null)
      reload()
    } catch (err) {
      handleError(err, tr("failed_to_update_device"))
    } finally {
      setBusy(null)
    }
  }

  async function handleUnbind() {
    if (!unbindTarget) return
    setBusy('device-unbind')
    setActionError(null)
    try {
      await unbindDevice(unbindTarget.id)
      setUnbindTarget(null)
      reload()
    } catch (err) {
      handleError(err, tr("failed_to_unbind_device"))
    } finally {
      setBusy(null)
    }
  }

  async function handleCreatePairing(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy('pairing-create')
    setPairingErrors({})
    setActionError(null)
    try {
      const response = await createPairingCode({
        user_email: pairingDraft.user_email.trim(),
        user_display_name: pairingDraft.user_display_name.trim() || undefined,
        expires_in_seconds: pairingDraft.ttl.trim() ? Number(pairingDraft.ttl) : undefined,
      })
      setPairingOpen(false)
      setCreatedCode(response.pairing_code)
      setPairingDraft(emptyPairing)
      reload()
    } catch (err) {
      if (err instanceof ApiError) {
        setPairingErrors(err.errors)
        setActionError(err.detail)
      } else {
        setActionError(tr("failed_to_create_pairing_code"))
      }
    } finally {
      setBusy(null)
    }
  }

  async function handleInvalidatePairing() {
    if (!invalidateTarget) return
    setBusy('pairing-invalidate')
    setActionError(null)
    try {
      await invalidatePairingCode(invalidateTarget.id)
      setInvalidateTarget(null)
      reload()
    } catch (err) {
      handleError(err, tr("failed_to_revoke_pairing_code"))
    } finally {
      setBusy(null)
    }
  }

  const deviceCount = data?.devices.length ?? 0
  const pairingCodeCount = data?.codes.length ?? 0

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr("devices_pairing")}
        desc={tr("devices_bind_users_to_keys_pairing_codes_handle_first_ti")}
        actions={
          <>
            {refresh}
            <button
              className="btn primary"
              type="button"
              onClick={() => {
                setPairingDraft(emptyPairing)
                setPairingErrors({})
                setActionError(null)
                setPairingOpen(true)
              }}
            >
              {tr("generate_pairing_code")}
            </button>
          </>
        }
      />

      <div className="tabs">
        <button className={tab === 'devices' ? 'active' : ''} type="button" onClick={() => setTab('devices')}>
          {tr("devices", { deviceCount: deviceCount })}
        </button>
        <button className={tab === 'pairing' ? 'active' : ''} type="button" onClick={() => setTab('pairing')}>
          {tr("pairing_codes", { pairingCodeCount: pairingCodeCount })}
        </button>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr("loading_devices_and_pairing_codes")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : tab === 'devices' ? (
        !data || data.devices.length === 0 ? (
          <EmptyPanel
            title={tr("no_devices_yet")}
            desc={tr("generate_a_pairing_code_for_an_employee_the_first_client")}
            action={
              <button className="btn primary" type="button" onClick={() => setPairingOpen(true)}>
                {tr("generate_pairing_code")}
              </button>
            }
          />
        ) : (
          <>
            <div className="card">
              <div className="toolbar">
                <div className="field" style={{ width: 160 }}>
                  <label htmlFor="device-status">{tr("status")}</label>
                  <select
                    id="device-status"
                    className="select"
                    value={statusFilter}
                    onChange={(event) => setStatusFilter(event.target.value)}
                  >
                    <option value="">{tr("all")}</option>
                    <option value="active">{tr("active")}</option>
                    <option value="disabled">{tr("disabled_2")}</option>
                  </select>
                </div>
              </div>
            </div>
            <div className="table-wrap">
              <table className="table dense">
                <thead>
                  <tr>
                    <th>{tr("device")}</th>
                    <th>{tr("user")}</th>
                    <th>{tr("platform_version")}</th>
                    <th>{tr("status")}</th>
                    <th>Key</th>
                    <th>{tr("last_seen")}</th>
                    <th className="cell-actions">{tr("actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {data.devices.map((device) => (
                    <tr key={device.id} className={device.status === 'disabled' ? 'is-off' : ''}>
                      <td>
                        <strong>{device.name}</strong>
                        <div className="hint">{device.last_ip || '—'}</div>
                      </td>
                      <td>
                        {device.user.display_name || device.user.email}
                        <div className="hint">{device.user.email}</div>
                      </td>
                      <td>
                        {device.platform}
                        <div className="hint">{device.client_version || '—'}</div>
                      </td>
                      <td>
                        <Badge tone={device.status === 'active' ? 'ok' : 'bad'}>
                          {device.status === 'active'
                            ? device.online
                              ? tr("online")
                              : tr("active")
                            : tr("disabled_2")}
                        </Badge>
                      </td>
                      <td>{device.key ? <code className="kbd">{device.key.key_prefix}</code> : '—'}</td>
                      <td>{device.last_seen_at ? new Date(device.last_seen_at).toLocaleString('zh-CN') : '—'}</td>
                      <td className="cell-actions">
                        <span className="row-actions">
                          <button className="btn sm" type="button" onClick={() => openEditDevice(device)}>
                            {tr("edit")}
                          </button>
                          <button className="btn sm danger" type="button" onClick={() => setUnbindTarget(device)}>
                            {tr("unbind")}
                          </button>
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </>
        )
      ) : !data || data.codes.length === 0 ? (
        <EmptyPanel
          title={tr("no_pairing_codes_yet")}
          desc={tr("pairing_codes_are_single_use_and_expire_after_10_minutes")}
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("user")}</th>
                <th>{tr("status")}</th>
                <th>{tr("expires")}</th>
                <th>{tr("created")}</th>
                <th className="cell-actions">{tr("actions")}</th>
              </tr>
            </thead>
            <tbody>
              {data.codes.map((code) => (
                <tr key={code.id}>
                  <td>
                    {code.user.display_name || code.user.email}
                    <div className="hint">{code.user.email}</div>
                  </td>
                  <td>{pairingStatusBadge(code.status, tr)}</td>
                  <td>{new Date(code.expires_at).toLocaleString('zh-CN')}</td>
                  <td>{new Date(code.created_at).toLocaleString('zh-CN')}</td>
                  <td className="cell-actions">
                    <button
                      className="btn sm danger"
                      type="button"
                      disabled={code.status !== 'active'}
                      onClick={() => setInvalidateTarget(code)}
                    >
                      {tr("revoke")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {tr("pairing_codes_are_not_shown_in_the_list_a_new_code_is_di")}
          </div>
        </div>
      )}

      {deviceEdit && (
        <Modal
          title={tr("edit_device")}
          sub={`${deviceEdit.user.email} · ${deviceEdit.platform}`}
          onClose={() => setDeviceEdit(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setDeviceEdit(null)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="edit-device-form" disabled={busy !== null}>
                {busy === 'device-save' ? tr("saving") : tr("save")}
              </button>
            </>
          }
        >
          <form id="edit-device-form" className="grid" onSubmit={(event) => void handleSaveDevice(event)} noValidate>
            <div className="field">
              <label htmlFor="device-name">{tr("device_name")}</label>
              <input
                id="device-name"
                className="input"
                value={deviceDraft.name}
                onChange={(event) => setDeviceDraft({ ...deviceDraft, name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="device-edit-status">{tr("status")}</label>
              <select
                id="device-edit-status"
                className="select"
                value={deviceDraft.status}
                onChange={(event) =>
                  setDeviceDraft({ ...deviceDraft, status: event.target.value as DeviceDraft['status'] })
                }
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

      {unbindTarget && (
        <ConfirmDialog
          title={tr("unbind_device", { name: unbindTarget.name })}
          desc={
            <p>
              {tr("the_device_token_key_becomes_invalid_after_unbinding_and")}
            </p>
          }
          confirmText={tr("unbind")}
          danger
          busy={busy === 'device-unbind'}
          onCancel={() => setUnbindTarget(null)}
          onConfirm={() => void handleUnbind()}
        />
      )}

      {pairingOpen && (
        <Modal
          title={tr("generate_pairing_code")}
          sub={tr("clients_register_with_the_server_address_and_pairing_cod")}
          onClose={() => setPairingOpen(false)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setPairingOpen(false)} disabled={busy !== null}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="pairing-form" disabled={busy !== null}>
                {busy === 'pairing-create' ? tr("generating") : tr("generate")}
              </button>
            </>
          }
        >
          <form id="pairing-form" className="grid" onSubmit={(event) => void handleCreatePairing(event)} noValidate>
            <div className="field">
              <label htmlFor="pairing-email">{tr("employee_email")}</label>
              <input
                id="pairing-email"
                className="input"
                type="email"
                placeholder="employee@corp.example.com"
                value={pairingDraft.user_email}
                onChange={(event) => setPairingDraft({ ...pairingDraft, user_email: event.target.value })}
              />
              {pairingErrors.user_email && <span className="field-error">{pairingErrors.user_email}</span>}
            </div>
            <div className="field">
              <label htmlFor="pairing-name">{tr("display_name_optional")}</label>
              <input
                id="pairing-name"
                className="input"
                value={pairingDraft.user_display_name}
                onChange={(event) => setPairingDraft({ ...pairingDraft, user_display_name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="pairing-ttl">{tr("ttl_seconds")}</label>
              <input
                id="pairing-ttl"
                className="input"
                type="number"
                min={60}
                value={pairingDraft.ttl}
                onChange={(event) => setPairingDraft({ ...pairingDraft, ttl: event.target.value })}
              />
              {pairingErrors.expires_in_seconds && (
                <span className="field-error">{pairingErrors.expires_in_seconds}</span>
              )}
            </div>
            {actionError && (
              <div className="form-error" role="alert">
                {actionError}
              </div>
            )}
          </form>
        </Modal>
      )}

      {invalidateTarget && (
        <ConfirmDialog
          title={tr("revoke_pairing_code")}
          desc={<p>{tr("unused_pairing_codes_become_invalid_immediately_used_rec")}</p>}
          confirmText={tr("revoke")}
          danger
          busy={busy === 'pairing-invalidate'}
          onCancel={() => setInvalidateTarget(null)}
          onConfirm={() => void handleInvalidatePairing()}
        />
      )}

      {createdCode && (
        <Modal
          title={tr("pairing_code_generated")}
          sub={tr("shown_only_once_share_it_with_the_employee_via_a_secure")}
          onClose={() => setCreatedCode(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setCreatedCode(null)}>
              {tr("close")}
            </button>
          }
        >
          <div className="field">
            <label>{tr("pairing_code")}</label>
            <div className="row">
              <input className="input" readOnly value={createdCode.code} style={{ fontFamily: 'var(--mono)' }} />
              <button className="btn" type="button" onClick={() => void navigator.clipboard?.writeText(createdCode.code)}>
                {tr("copy")}
              </button>
            </div>
            <span className="hint">
              {createdCode.user.email} · {tr("valid_until")}{' '}
              {new Date(createdCode.expires_at).toLocaleString('zh-CN')}
            </span>
          </div>
        </Modal>
      )}
    </>
  )
}
