import { useState } from 'react'
import { ApiError } from '../api/client'
import { listDevices, listKeys, listUserGroups, listUsers } from '../api/accessApi'
import { getCatalogPreview } from '../api/preview'
import type { CatalogPreview } from '../api/preview'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { ErrorPanel, LoadingPanel } from '../components/ScreenState'

export function CatalogPreviewPage() {
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
      setPreviewError('请选择员工')
      return
    }
    setBusy(true)
    setPreviewError(null)
    try {
      setPreview(await getCatalogPreview(userId, deviceId || undefined, keyId || undefined))
    } catch (err) {
      setPreviewError(err instanceof ApiError ? err.detail : '预览失败')
      setPreview(null)
    } finally {
      setBusy(false)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead title="员工目录预览" desc="按员工/设备/Key 预览实际可见、可用模型与工具默认。" />
        <LoadingPanel title="正在读取员工与设备…" />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead title="员工目录预览" desc="按员工/设备/Key 预览实际可见、可用模型与工具默认。" />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  return (
    <>
      <PageHead
        title="员工目录预览"
        desc="按员工/设备/Key 预览实际可见、可用模型与工具默认；不含价格/供应商/canonical。"
        actions={
          <button className="btn primary" type="button" onClick={() => void handlePreview()} disabled={busy}>
            {busy ? '预览中…' : '预览目录'}
          </button>
        }
      />

      <div className="card">
        <div className="grid cols-3">
          <div className="field">
            <label htmlFor="preview-user">员工</label>
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
              <option value="">选择员工</option>
              {data.users.map((user) => (
                <option key={user.id} value={user.id}>
                  {user.display_name || user.email} · {user.email}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="preview-device">设备（可选）</label>
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
              <option value="">不指定设备</option>
              {userDevices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name} · {device.status}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="preview-key">Key（可选）</label>
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
              <option value="">不指定 Key</option>
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
            选择员工后可预览该员工实际目录；设备/Key 可选，用于验证白名单/设备维度。
          </p>
        )}
      </div>

      {preview && (
        <>
          <div className="grid cols-2">
            <div className="card">
              <div className="row between">
                <h3>预览对象</h3>
                <Badge tone="accent">{preview.catalog_version}</Badge>
              </div>
              <dl className="kv" style={{ marginTop: 10 }}>
                <dt>员工</dt>
                <dd>
                  {preview.subject.display_name || preview.subject.email} · {preview.subject.email}
                </dd>
                <dt>状态</dt>
                <dd>{preview.subject.user_status}</dd>
                <dt>设备</dt>
                <dd>{preview.subject.device_name ?? '未指定'}</dd>
                <dt>Key 白名单</dt>
                <dd>{preview.subject.key_allowed_models.join(', ') || '不限'}</dd>
                <dt>用户组</dt>
                <dd>{preview.subject.user_group_ids.length} 个</dd>
              </dl>
            </div>
            <div className="card">
              <div className="row between">
                <h3>强制标注</h3>
                <Badge tone="warn">{preview.models.length} 个可见模型</Badge>
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
              <h3>可见模型</h3>
              <span className="sub">只投影 visible；allowed 供调用侧判断</span>
            </div>
            <div className="table-wrap" style={{ marginTop: 10 }}>
              <table className="table dense">
                <thead>
                  <tr>
                    <th>模型</th>
                    <th>显示名</th>
                    <th>上游模型</th>
                    <th>可见</th>
                    <th>可用</th>
                    <th>命中对象</th>
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
                        <Badge tone="ok">可见</Badge>
                      </td>
                      <td>
                        <Badge tone={model.allowed ? 'ok' : 'warn'}>{model.allowed ? '可用' : '不可用'}</Badge>
                      </td>
                      <td>{model.matched_by ?? '—'}</td>
                    </tr>
                  ))}
                  {preview.models.length === 0 && (
                    <tr>
                      <td colSpan={6} className="hint">
                        该员工没有可见模型；检查发布对象/可见状态/Key 白名单。
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>

          <div className="card">
            <div className="row between">
              <h3>工具默认</h3>
              <span className="sub">默认/可切换均需可见且可用</span>
            </div>
            <div className="table-wrap" style={{ marginTop: 10 }}>
              <table className="table dense">
                <thead>
                  <tr>
                    <th>工具</th>
                    <th>命中层级</th>
                    <th>默认模型</th>
                    <th>可切换</th>
                    <th>告警</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.tool_defaults.map((tool) => (
                    <tr key={tool.tool_key}>
                      <td>
                        <strong>{tool.tool_key}</strong>
                      </td>
                      <td>{tool.source}</td>
                      <td>{tool.default_model ?? '无有效默认'}</td>
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
                        没有工具默认；工具将使用自身默认或提示管理员配置。
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
