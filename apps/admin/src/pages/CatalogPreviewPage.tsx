import { useState } from 'react'
import { ApiError } from '../api/client'
import { listDevices, listKeys, listUserGroups, listUsers } from '../api/accessApi'
import { getCatalogPreview } from '../api/preview'
import type { CatalogPreview } from '../api/preview'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { useI18n } from '../i18n/I18nProvider'

export function CatalogPreviewPage() {
  const { tr } = useI18n()
  const { data, loading, error, reload } = useAsync(
    () =>
      Promise.all([listUsers(200), listDevices(200), listKeys(200), listUserGroups(200)]).then(
        ([users, devices, keys, groups]) => ({
          users: users.items,
          devices: devices.items,
          keys: keys.items,
          groups: groups.items,
        }),
      ),
    [],
  )

  const [userId, setUserId] = useState('')
  const [deviceId, setDeviceId] = useState('')
  const [keyId, setKeyId] = useState('')
  const [preview, setPreview] = useState<CatalogPreview | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const userDevices = data?.devices.filter((device) => device.user.id === userId) ?? []
  const userKeys = data?.keys.filter((key) => key.user.id === userId) ?? []
  const selectedUser = data?.users.find((user) => user.id === userId)

  async function handlePreview() {
    if (!userId) {
      setPreviewError(tr('请选择员工', 'Select an employee first'))
      return
    }
    setBusy(true)
    setPreviewError(null)
    try {
      setPreview(await getCatalogPreview(userId, deviceId || undefined, keyId || undefined))
    } catch (err) {
      setPreviewError(err instanceof ApiError ? err.detail : tr('预览失败', 'Preview failed'))
      setPreview(null)
    } finally {
      setBusy(false)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead
          title={tr('员工目录预览', 'Employee catalog preview')}
          desc={tr(
            '按员工/设备/Key 预览实际可见、可用模型与工具默认。',
            'Preview the models and tool defaults actually visible and allowed for an employee, device, or key.',
          )}
        />
        <LoadingPanel title={tr('正在读取员工与设备…', 'Loading employees and devices…')} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead
          title={tr('员工目录预览', 'Employee catalog preview')}
          desc={tr(
            '按员工/设备/Key 预览实际可见、可用模型与工具默认。',
            'Preview the models and tool defaults actually visible and allowed for an employee, device, or key.',
          )}
        />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  return (
    <>
      <PageHead
        title={tr('员工目录预览', 'Employee catalog preview')}
        desc={tr(
          '按员工/设备/Key 预览实际可见、可用模型与工具默认；不含价格/供应商/canonical。',
          'Preview the models and tool defaults actually visible and allowed for an employee, device, or key; excludes pricing, provider, and canonical.',
        )}
        actions={
          <button className="btn primary" type="button" onClick={() => void handlePreview()} disabled={busy}>
            {busy ? tr('预览中…', 'Previewing…') : tr('预览目录', 'Preview catalog')}
          </button>
        }
      />

      <div className="card">
        <div className="grid cols-3">
          <div className="field">
            <label htmlFor="preview-user">{tr('员工', 'Employee')}</label>
            <select
              id="preview-user"
              className="select"
              value={userId}
              onChange={(event) => {
                setUserId(event.target.value)
                setDeviceId('')
                setKeyId('')
                setPreview(null)
                setPreviewError(null)
              }}
            >
              <option value="">{tr('选择员工', 'Select employee')}</option>
              {data.users.map((user) => (
                <option key={user.id} value={user.id}>
                  {user.display_name || user.email} · {user.email}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="preview-device">{tr('设备（可选）', 'Device (optional)')}</label>
            <select
              id="preview-device"
              className="select"
              value={deviceId}
              onChange={(event) => {
                setDeviceId(event.target.value)
                setPreview(null)
              }}
              disabled={!userId}
            >
              <option value="">{tr('不指定设备', 'Any device')}</option>
              {userDevices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name} · {device.status}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="preview-key">{tr('Key（可选）', 'Key (optional)')}</label>
            <select
              id="preview-key"
              className="select"
              value={keyId}
              onChange={(event) => {
                setKeyId(event.target.value)
                setPreview(null)
              }}
              disabled={!userId}
            >
              <option value="">{tr('不指定 Key', 'Any key')}</option>
              {userKeys.map((key) => (
                <option key={key.id} value={key.id}>
                  {key.key_prefix} · {key.status}
                </option>
              ))}
            </select>
          </div>
        </div>
        {previewError && (
          <div className="form-error" role="alert" style={{ marginTop: 12 }}>
            {previewError}
          </div>
        )}
        {!preview && !previewError && (
          <p className="hint" style={{ marginTop: 10 }}>
            {tr(
              '选择员工后可预览该员工实际目录；设备/Key 可选，用于验证白名单/设备维度。',
              'Select an employee to preview their effective catalog. Device and key are optional and used to verify allowlists and device scoping.',
            )}
          </p>
        )}
      </div>

      {preview && (
        <>
          <div className="grid cols-2">
            <div className="card">
              <div className="row between">
                <h3>{tr('预览对象', 'Preview subject')}</h3>
                <Badge tone="accent">{preview.catalog_version}</Badge>
              </div>
              <dl className="kv" style={{ marginTop: 10 }}>
                <dt>{tr('员工', 'Employee')}</dt>
                <dd>
                  {preview.subject.display_name || preview.subject.email} · {preview.subject.email}
                </dd>
                <dt>{tr('状态', 'Status')}</dt>
                <dd>{preview.subject.user_status}</dd>
                <dt>{tr('设备', 'Device')}</dt>
                <dd>{preview.subject.device_name ?? tr('未指定', 'Not specified')}</dd>
                <dt>{tr('Key 白名单', 'Key allowlist')}</dt>
                <dd>{preview.subject.key_allowed_models.join(', ') || tr('不限', 'Unrestricted')}</dd>
                <dt>{tr('用户组', 'User groups')}</dt>
                <dd>
                  {tr(
                    `${preview.subject.user_group_ids.length} 个`,
                    `${preview.subject.user_group_ids.length} groups`,
                  )}
                </dd>
              </dl>
            </div>
            <div className="card">
              <div className="row between">
                <h3>{tr('强制标注', 'Enforcement')}</h3>
                <Badge tone="warn">
                  {tr(`${preview.models.length} 个可见模型`, `${preview.models.length} visible models`)}
                </Badge>
              </div>
              <div style={{ marginTop: 10 }}>
                <div className="status-row">
                  <span>visible</span>
                  <span style={{ marginLeft: 'auto' }}>{preview.enforcement.visible}</span>
                </div>
                <div className="status-row">
                  <span>allowed</span>
                  <span style={{ marginLeft: 'auto' }}>{preview.enforcement.allowed}</span>
                </div>
                <div className="status-row">
                  <span>quota / data_level</span>
                  <span style={{ marginLeft: 'auto' }}>
                    {preview.enforcement.quota} / {preview.enforcement.data_level}
                  </span>
                </div>
              </div>
              {preview.warnings.length > 0 && (
                <p className="note warn" style={{ marginTop: 10 }}>
                  {preview.warnings.join(' · ')}
                </p>
              )}
            </div>
          </div>

          <div className="card">
            <div className="row between">
              <h3>{tr('可见模型', 'Visible models')}</h3>
              <span className="sub">
                {tr(
                  '只投影 visible；allowed 供调用侧判断',
                  'Only visible models are projected; allowed is evaluated on the caller side',
                )}
              </span>
            </div>
            <div className="table-wrap" style={{ marginTop: 10 }}>
              <table className="table dense">
                <thead>
                  <tr>
                    <th>{tr('模型', 'Model')}</th>
                    <th>{tr('显示名', 'Display name')}</th>
                    <th>{tr('上游模型', 'Upstream model')}</th>
                    <th>{tr('可见', 'Visible')}</th>
                    <th>{tr('可用', 'Allowed')}</th>
                    <th>{tr('命中对象', 'Matched via')}</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.models.map((model) => (
                    <tr key={model.id}>
                      <td>
                        <strong>{model.id}</strong>
                      </td>
                      <td>{model.display_name}</td>
                      <td>{model.upstream_model ?? '—'}</td>
                      <td>
                        <Badge tone="ok">{tr('可见', 'Visible')}</Badge>
                      </td>
                      <td>
                        <Badge tone={model.allowed ? 'ok' : 'warn'}>
                          {model.allowed ? tr('可用', 'Allowed') : tr('不可用', 'Not allowed')}
                        </Badge>
                      </td>
                      <td>{model.matched_by ?? '—'}</td>
                    </tr>
                  ))}
                  {preview.models.length === 0 && (
                    <tr>
                      <td colSpan={6} className="hint">
                        {tr(
                          '该员工没有可见模型；检查发布对象/可见状态/Key 白名单。',
                          'No visible models for this employee. Check publication targets, visibility, and the key allowlist.',
                        )}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>

          <div className="card">
            <div className="row between">
              <h3>{tr('工具默认', 'Tool defaults')}</h3>
              <span className="sub">
                {tr('默认/可切换均需可见且可用', 'Both default and switchable models must be visible and allowed')}
              </span>
            </div>
            <div className="table-wrap" style={{ marginTop: 10 }}>
              <table className="table dense">
                <thead>
                  <tr>
                    <th>{tr('工具', 'Tool')}</th>
                    <th>{tr('命中层级', 'Matched level')}</th>
                    <th>{tr('默认模型', 'Default model')}</th>
                    <th>{tr('可切换', 'Switchable')}</th>
                    <th>{tr('告警', 'Warnings')}</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.tool_defaults.map((tool) => (
                    <tr key={tool.tool_key}>
                      <td>
                        <strong>{tool.tool_key}</strong>
                      </td>
                      <td>{tool.source}</td>
                      <td>{tool.default_model ?? tr('无有效默认', 'No valid default')}</td>
                      <td>{tool.switchable_models.join(', ') || '—'}</td>
                      <td>
                        {tool.warnings.length === 0 ? (
                          <span className="hint">—</span>
                        ) : (
                          <Badge tone="warn">{tool.warnings.join(', ')}</Badge>
                        )}
                      </td>
                    </tr>
                  ))}
                  {preview.tool_defaults.length === 0 && (
                    <tr>
                      <td colSpan={5} className="hint">
                        {tr(
                          '没有工具默认；工具将使用自身默认或提示管理员配置。',
                          'No tool defaults; tools fall back to their own defaults or prompt the admin to configure them.',
                        )}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
        </>
      )}
    </>
  )
}
