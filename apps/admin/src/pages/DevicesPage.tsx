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

function pairingStatusBadge(status: string, tr: (zh: string, en: string) => string) {
  if (status === 'active') return <Badge tone="ok">{tr('未使用', 'Unused')}</Badge>
  if (status === 'used') return <Badge>{tr('已使用', 'Used')}</Badge>
  if (status === 'expired') return <Badge tone="warn">{tr('已过期', 'Expired')}</Badge>
  return <Badge tone="warn">{tr('已作废', 'Revoked')}</Badge>
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
      handleError(err, tr('更新设备失败', 'Failed to update device'))
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
      handleError(err, tr('解绑设备失败', 'Failed to unbind device'))
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
        setActionError(tr('创建配对码失败', 'Failed to create pairing code'))
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
      handleError(err, tr('作废配对码失败', 'Failed to revoke pairing code'))
    } finally {
      setBusy(null)
    }
  }

  const deviceCount = data?.devices.length ?? 0
  const pairingCodeCount = data?.codes.length ?? 0

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr('设备与配对', 'Devices & pairing')}
        desc={tr(
          '设备绑定用户与 Key；配对码用于客户端首次注册。',
          'Devices bind users to keys; pairing codes handle first-time client registration.',
        )}
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
              {tr('生成配对码', 'Generate pairing code')}
            </button>
          </>
        }
      />

      <div className="tabs">
        <button className={tab === 'devices' ? 'active' : ''} type="button" onClick={() => setTab('devices')}>
          {tr(`设备（${deviceCount}）`, `Devices (${deviceCount})`)}
        </button>
        <button className={tab === 'pairing' ? 'active' : ''} type="button" onClick={() => setTab('pairing')}>
          {tr(`配对码（${pairingCodeCount}）`, `Pairing codes (${pairingCodeCount})`)}
        </button>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取设备与配对码…', 'Loading devices and pairing codes…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : tab === 'devices' ? (
        !data || data.devices.length === 0 ? (
          <EmptyPanel
            title={tr('还没有设备', 'No devices yet')}
            desc={tr(
              '生成配对码发给员工，客户端首次配对后会创建用户/设备/虚拟 Key。',
              'Generate a pairing code for an employee; the first client pairing creates the user, device, and virtual key.',
            )}
            action={
              <button className="btn primary" type="button" onClick={() => setPairingOpen(true)}>
                {tr('生成配对码', 'Generate pairing code')}
              </button>
            }
          />
        ) : (
          <>
            <div className="card">
              <div className="toolbar">
                <div className="field" style={{ width: 160 }}>
                  <label htmlFor="device-status">{tr('状态', 'Status')}</label>
                  <select
                    id="device-status"
                    className="select"
                    value={statusFilter}
                    onChange={(event) => setStatusFilter(event.target.value)}
                  >
                    <option value="">{tr('全部', 'All')}</option>
                    <option value="active">{tr('正常', 'Active')}</option>
                    <option value="disabled">{tr('已禁用', 'Disabled')}</option>
                  </select>
                </div>
              </div>
            </div>
            <div className="table-wrap">
              <table className="table dense">
                <thead>
                  <tr>
                    <th>{tr('设备', 'Device')}</th>
                    <th>{tr('用户', 'User')}</th>
                    <th>{tr('平台 / 版本', 'Platform / version')}</th>
                    <th>{tr('状态', 'Status')}</th>
                    <th>Key</th>
                    <th>{tr('最近在线', 'Last seen')}</th>
                    <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                              ? tr('在线', 'Online')
                              : tr('正常', 'Active')
                            : tr('已禁用', 'Disabled')}
                        </Badge>
                      </td>
                      <td>{device.key ? <code className="kbd">{device.key.key_prefix}</code> : '—'}</td>
                      <td>{device.last_seen_at ? new Date(device.last_seen_at).toLocaleString('zh-CN') : '—'}</td>
                      <td className="cell-actions">
                        <span className="row-actions">
                          <button className="btn sm" type="button" onClick={() => openEditDevice(device)}>
                            {tr('编辑', 'Edit')}
                          </button>
                          <button className="btn sm danger" type="button" onClick={() => setUnbindTarget(device)}>
                            {tr('解绑', 'Unbind')}
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
          title={tr('还没有配对码', 'No pairing codes yet')}
          desc={tr('配对码一次性使用，默认 10 分钟过期。', 'Pairing codes are single-use and expire after 10 minutes by default.')}
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('用户', 'User')}</th>
                <th>{tr('状态', 'Status')}</th>
                <th>{tr('过期', 'Expires')}</th>
                <th>{tr('创建', 'Created')}</th>
                <th className="cell-actions">{tr('操作', 'Actions')}</th>
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
                      {tr('作废', 'Revoke')}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            {tr(
              '列表不回显配对码明文；新码只在生成时显示一次。',
              'Pairing codes are not shown in the list; a new code is displayed only once.',
            )}
          </div>
        </div>
      )}

      {deviceEdit && (
        <Modal
          title={tr('编辑设备', 'Edit device')}
          sub={`${deviceEdit.user.email} · ${deviceEdit.platform}`}
          onClose={() => setDeviceEdit(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setDeviceEdit(null)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="edit-device-form" disabled={busy !== null}>
                {busy === 'device-save' ? tr('保存中…', 'Saving…') : tr('保存', 'Save')}
              </button>
            </>
          }
        >
          <form id="edit-device-form" className="grid" onSubmit={(event) => void handleSaveDevice(event)} noValidate>
            <div className="field">
              <label htmlFor="device-name">{tr('设备名', 'Device name')}</label>
              <input
                id="device-name"
                className="input"
                value={deviceDraft.name}
                onChange={(event) => setDeviceDraft({ ...deviceDraft, name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="device-edit-status">{tr('状态', 'Status')}</label>
              <select
                id="device-edit-status"
                className="select"
                value={deviceDraft.status}
                onChange={(event) =>
                  setDeviceDraft({ ...deviceDraft, status: event.target.value as DeviceDraft['status'] })
                }
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

      {unbindTarget && (
        <ConfirmDialog
          title={tr(`解绑设备 ${unbindTarget.name}？`, `Unbind device ${unbindTarget.name}?`)}
          desc={
            <p>
              {tr(
                '解绑后该设备 token/Key 失效，需重新配对；操作写入审计。',
                'The device token/key becomes invalid after unbinding and re-pairing is required; this action is written to the audit log.',
              )}
            </p>
          }
          confirmText={tr('解绑', 'Unbind')}
          danger
          busy={busy === 'device-unbind'}
          onCancel={() => setUnbindTarget(null)}
          onConfirm={() => void handleUnbind()}
        />
      )}

      {pairingOpen && (
        <Modal
          title={tr('生成配对码', 'Generate pairing code')}
          sub={tr('客户端用服务器地址 + 配对码完成注册', 'Clients register with the server address and pairing code')}
          onClose={() => setPairingOpen(false)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setPairingOpen(false)} disabled={busy !== null}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="pairing-form" disabled={busy !== null}>
                {busy === 'pairing-create' ? tr('生成中…', 'Generating…') : tr('生成', 'Generate')}
              </button>
            </>
          }
        >
          <form id="pairing-form" className="grid" onSubmit={(event) => void handleCreatePairing(event)} noValidate>
            <div className="field">
              <label htmlFor="pairing-email">{tr('员工邮箱', 'Employee email')}</label>
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
              <label htmlFor="pairing-name">{tr('显示名（可选）', 'Display name (optional)')}</label>
              <input
                id="pairing-name"
                className="input"
                value={pairingDraft.user_display_name}
                onChange={(event) => setPairingDraft({ ...pairingDraft, user_display_name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="pairing-ttl">{tr('有效期（秒）', 'TTL (seconds)')}</label>
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
          title={tr('作废配对码？', 'Revoke pairing code?')}
          desc={<p>{tr('未使用的配对码立即失效；已使用的记录保留。', 'Unused pairing codes become invalid immediately; used records are kept.')}</p>}
          confirmText={tr('作废', 'Revoke')}
          danger
          busy={busy === 'pairing-invalidate'}
          onCancel={() => setInvalidateTarget(null)}
          onConfirm={() => void handleInvalidatePairing()}
        />
      )}

      {createdCode && (
        <Modal
          title={tr('配对码已生成', 'Pairing code generated')}
          sub={tr('仅显示一次；请通过安全渠道发给员工', 'Shown only once; share it with the employee via a secure channel')}
          onClose={() => setCreatedCode(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setCreatedCode(null)}>
              {tr('关闭', 'Close')}
            </button>
          }
        >
          <div className="field">
            <label>{tr('配对码', 'Pairing code')}</label>
            <div className="row">
              <input className="input" readOnly value={createdCode.code} style={{ fontFamily: 'var(--mono)' }} />
              <button className="btn" type="button" onClick={() => void navigator.clipboard?.writeText(createdCode.code)}>
                {tr('复制', 'Copy')}
              </button>
            </div>
            <span className="hint">
              {createdCode.user.email} · {tr('有效期至', 'Valid until')}{' '}
              {new Date(createdCode.expires_at).toLocaleString('zh-CN')}
            </span>
          </div>
        </Modal>
      )}
    </>
  )
}
