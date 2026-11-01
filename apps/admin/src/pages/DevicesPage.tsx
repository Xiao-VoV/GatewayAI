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

function pairingStatusBadge(status: string) {
  if (status === 'active') return <Badge tone="ok">未使用</Badge>
  if (status === 'used') return <Badge>已使用</Badge>
  if (status === 'expired') return <Badge tone="warn">已过期</Badge>
  return <Badge tone="warn">已作废</Badge>
}

export function DevicesPage() {
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
      handleError(err, '更新设备失败')
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
      handleError(err, '解绑设备失败')
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
        setActionError('创建配对码失败')
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
      handleError(err, '作废配对码失败')
    } finally {
      setBusy(null)
    }
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? '刷新中…' : '刷新'}
    </button>
  )

  return (
    <>
      <PageHead
        title="设备与配对"
        desc="设备绑定用户与 Key；配对码用于客户端首次注册。"
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
              生成配对码
            </button>
          </>
        }
      />

      <div className="tabs">
        <button className={tab === 'devices' ? 'active' : ''} type="button" onClick={() => setTab('devices')}>
          设备（{data?.devices.length ?? 0}）
        </button>
        <button className={tab === 'pairing' ? 'active' : ''} type="button" onClick={() => setTab('pairing')}>
          配对码（{data?.codes.length ?? 0}）
        </button>
      </div>

      {actionError && (
        <div className="form-error" role="alert">
          {actionError}
        </div>
      )}

      {loading && !data ? (
        <LoadingPanel title="正在读取设备与配对码…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : tab === 'devices' ? (
        !data || data.devices.length === 0 ? (
          <EmptyPanel
            title="还没有设备"
            desc="生成配对码发给员工，客户端首次配对后会创建用户/设备/虚拟 Key。"
            action={
              <button className="btn primary" type="button" onClick={() => setPairingOpen(true)}>
                生成配对码
              </button>
            }
          />
        ) : (
          <>
            <div className="card">
              <div className="toolbar">
                <div className="field" style={{ width: 160 }}>
                  <label htmlFor="device-status">状态</label>
                  <select
                    id="device-status"
                    className="select"
                    value={statusFilter}
                    onChange={(event) => setStatusFilter(event.target.value)}
                  >
                    <option value="">全部</option>
                    <option value="active">正常</option>
                    <option value="disabled">已禁用</option>
                  </select>
                </div>
              </div>
            </div>
            <div className="table-wrap">
              <table className="table dense">
                <thead>
                  <tr>
                    <th>设备</th>
                    <th>用户</th>
                    <th>平台 / 版本</th>
                    <th>状态</th>
                    <th>Key</th>
                    <th>最近在线</th>
                    <th className="cell-actions">操作</th>
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
                          {device.status === 'active' ? (device.online ? '在线' : '正常') : '已禁用'}
                        </Badge>
                      </td>
                      <td>{device.key ? <code className="kbd">{device.key.key_prefix}</code> : '—'}</td>
                      <td>{device.last_seen_at ? new Date(device.last_seen_at).toLocaleString('zh-CN') : '—'}</td>
                      <td className="cell-actions">
                        <span className="row-actions">
                          <button className="btn sm" type="button" onClick={() => openEditDevice(device)}>
                            编辑
                          </button>
                          <button className="btn sm danger" type="button" onClick={() => setUnbindTarget(device)}>
                            解绑
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
        <EmptyPanel title="还没有配对码" desc="配对码一次性使用，默认 10 分钟过期。" />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>用户</th>
                <th>状态</th>
                <th>过期</th>
                <th>创建</th>
                <th className="cell-actions">操作</th>
              </tr>
            </thead>
            <tbody>
              {data.codes.map((code) => (
                <tr key={code.id}>
                  <td>
                    {code.user.display_name || code.user.email}
                    <div className="hint">{code.user.email}</div>
                  </td>
                  <td>{pairingStatusBadge(code.status)}</td>
                  <td>{new Date(code.expires_at).toLocaleString('zh-CN')}</td>
                  <td>{new Date(code.created_at).toLocaleString('zh-CN')}</td>
                  <td className="cell-actions">
                    <button
                      className="btn sm danger"
                      type="button"
                      disabled={code.status !== 'active'}
                      onClick={() => setInvalidateTarget(code)}
                    >
                      作废
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            列表不回显配对码明文；新码只在生成时显示一次。
          </div>
        </div>
      )}

      {deviceEdit && (
        <Modal
          title="编辑设备"
          sub={`${deviceEdit.user.email} · ${deviceEdit.platform}`}
          onClose={() => setDeviceEdit(null)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setDeviceEdit(null)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="edit-device-form" disabled={busy !== null}>
                {busy === 'device-save' ? '保存中…' : '保存'}
              </button>
            </>
          }
        >
          <form id="edit-device-form" className="grid" onSubmit={(event) => void handleSaveDevice(event)} noValidate>
            <div className="field">
              <label htmlFor="device-name">设备名</label>
              <input
                id="device-name"
                className="input"
                value={deviceDraft.name}
                onChange={(event) => setDeviceDraft({ ...deviceDraft, name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="device-edit-status">状态</label>
              <select
                id="device-edit-status"
                className="select"
                value={deviceDraft.status}
                onChange={(event) =>
                  setDeviceDraft({ ...deviceDraft, status: event.target.value as DeviceDraft['status'] })
                }
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

      {unbindTarget && (
        <ConfirmDialog
          title={`解绑设备 ${unbindTarget.name}？`}
          desc={<p>解绑后该设备 token/Key 失效，需重新配对；操作写入审计。</p>}
          confirmText="解绑"
          danger
          busy={busy === 'device-unbind'}
          onCancel={() => setUnbindTarget(null)}
          onConfirm={() => void handleUnbind()}
        />
      )}

      {pairingOpen && (
        <Modal
          title="生成配对码"
          sub="客户端用服务器地址 + 配对码完成注册"
          onClose={() => setPairingOpen(false)}
          footer={
            <>
              <button className="btn" type="button" onClick={() => setPairingOpen(false)} disabled={busy !== null}>
                取消
              </button>
              <button className="btn primary" type="submit" form="pairing-form" disabled={busy !== null}>
                {busy === 'pairing-create' ? '生成中…' : '生成'}
              </button>
            </>
          }
        >
          <form id="pairing-form" className="grid" onSubmit={(event) => void handleCreatePairing(event)} noValidate>
            <div className="field">
              <label htmlFor="pairing-email">员工邮箱</label>
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
              <label htmlFor="pairing-name">显示名（可选）</label>
              <input
                id="pairing-name"
                className="input"
                value={pairingDraft.user_display_name}
                onChange={(event) => setPairingDraft({ ...pairingDraft, user_display_name: event.target.value })}
              />
            </div>
            <div className="field">
              <label htmlFor="pairing-ttl">有效期（秒）</label>
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
          title="作废配对码？"
          desc={<p>未使用的配对码立即失效；已使用的记录保留。</p>}
          confirmText="作废"
          danger
          busy={busy === 'pairing-invalidate'}
          onCancel={() => setInvalidateTarget(null)}
          onConfirm={() => void handleInvalidatePairing()}
        />
      )}

      {createdCode && (
        <Modal
          title="配对码已生成"
          sub="仅显示一次；请通过安全渠道发给员工"
          onClose={() => setCreatedCode(null)}
          footer={
            <button className="btn primary" type="button" onClick={() => setCreatedCode(null)}>
              关闭
            </button>
          }
        >
          <div className="field">
            <label>配对码</label>
            <div className="row">
              <input className="input" readOnly value={createdCode.code} style={{ fontFamily: 'var(--mono)' }} />
              <button className="btn" type="button" onClick={() => void navigator.clipboard?.writeText(createdCode.code)}>
                复制
              </button>
            </div>
            <span className="hint">
              {createdCode.user.email} · 有效期至 {new Date(createdCode.expires_at).toLocaleString('zh-CN')}
            </span>
          </div>
        </Modal>
      )}
    </>
  )
}
