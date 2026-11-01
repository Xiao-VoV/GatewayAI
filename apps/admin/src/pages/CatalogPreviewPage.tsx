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
      setPreviewError(tr("select_an_employee_first"))
      return
    }
    setBusy(true)
    setPreviewError(null)
    try {
      setPreview(await getCatalogPreview(userId, deviceId || undefined, keyId || undefined))
    } catch (err) {
      setPreviewError(err instanceof ApiError ? err.detail : tr("preview_failed"))
      setPreview(null)
    } finally {
      setBusy(false)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead
          title={tr("employee_catalog_preview")}
          desc={tr("preview_the_models_and_tool_defaults_actually_visible_an_2")}
        />
        <LoadingPanel title={tr("loading_employees_and_devices")} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead
          title={tr("employee_catalog_preview")}
          desc={tr("preview_the_models_and_tool_defaults_actually_visible_an_2")}
        />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!data) return null

  return (
    <>
      <PageHead
        title={tr("employee_catalog_preview")}
        desc={tr("preview_the_models_and_tool_defaults_actually_visible_an")}
        actions={
          <button className="btn primary" type="button" onClick={() => void handlePreview()} disabled={busy}>
            {busy ? tr("previewing") : tr("preview_catalog")}
          </button>
        }
      />

      <div className="card">
        <div className="grid cols-3">
          <div className="field">
            <label htmlFor="preview-user">{tr("employee")}</label>
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
              <option value="">{tr("select_employee")}</option>
              {data.users.map((user) => (
                <option key={user.id} value={user.id}>
                  {user.display_name || user.email} · {user.email}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="preview-device">{tr("device_optional")}</label>
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
              <option value="">{tr("any_device")}</option>
              {userDevices.map((device) => (
                <option key={device.id} value={device.id}>
                  {device.name} · {device.status}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label htmlFor="preview-key">{tr("key_optional")}</label>
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
              <option value="">{tr("any_key")}</option>
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
            {tr("select_an_employee_to_preview_their_effective_catalog_de")}
          </p>
        )}
      </div>

      {preview && (
        <>
          <div className="grid cols-2">
            <div className="card">
              <div className="row between">
                <h3>{tr("preview_subject")}</h3>
                <Badge tone="accent">{preview.catalog_version}</Badge>
              </div>
              <dl className="kv" style={{ marginTop: 10 }}>
                <dt>{tr("employee")}</dt>
                <dd>
                  {preview.subject.display_name || preview.subject.email} · {preview.subject.email}
                </dd>
                <dt>{tr("status")}</dt>
                <dd>{preview.subject.user_status}</dd>
                <dt>{tr("device")}</dt>
                <dd>{preview.subject.device_name ?? tr("not_specified")}</dd>
                <dt>{tr("key_allowlist")}</dt>
                <dd>{preview.subject.key_allowed_models.join(', ') || tr("unrestricted")}</dd>
                <dt>{tr("user_groups")}</dt>
                <dd>
                  {tr("groups", { length: preview.subject.user_group_ids.length })}
                </dd>
              </dl>
            </div>
            <div className="card">
              <div className="row between">
                <h3>{tr("enforcement")}</h3>
                <Badge tone="warn">
                  {tr("visible_models_2", { length: preview.models.length })}
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
              <h3>{tr("visible_models")}</h3>
              <span className="sub">
                {tr("only_visible_models_are_projected_allowed_is_evaluated_o")}
              </span>
            </div>
            <div className="table-wrap" style={{ marginTop: 10 }}>
              <table className="table dense">
                <thead>
                  <tr>
                    <th>{tr("model")}</th>
                    <th>{tr("display_name")}</th>
                    <th>{tr("upstream_model")}</th>
                    <th>{tr("visible")}</th>
                    <th>{tr("allowed")}</th>
                    <th>{tr("matched_via")}</th>
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
                        <Badge tone="ok">{tr("visible")}</Badge>
                      </td>
                      <td>
                        <Badge tone={model.allowed ? 'ok' : 'warn'}>
                          {model.allowed ? tr("allowed") : tr("not_allowed")}
                        </Badge>
                      </td>
                      <td>{model.matched_by ?? '—'}</td>
                    </tr>
                  ))}
                  {preview.models.length === 0 && (
                    <tr>
                      <td colSpan={6} className="hint">
                        {tr("no_visible_models_for_this_employee_check_publication_ta")}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>

          <div className="card">
            <div className="row between">
              <h3>{tr("tool_defaults")}</h3>
              <span className="sub">
                {tr("both_default_and_switchable_models_must_be_visible_and_a")}
              </span>
            </div>
            <div className="table-wrap" style={{ marginTop: 10 }}>
              <table className="table dense">
                <thead>
                  <tr>
                    <th>{tr("tool")}</th>
                    <th>{tr("matched_level")}</th>
                    <th>{tr("default_model")}</th>
                    <th>{tr("switchable")}</th>
                    <th>{tr("warnings")}</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.tool_defaults.map((tool) => (
                    <tr key={tool.tool_key}>
                      <td>
                        <strong>{tool.tool_key}</strong>
                      </td>
                      <td>{tool.source}</td>
                      <td>{tool.default_model ?? tr("no_valid_default")}</td>
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
                        {tr("no_tool_defaults_tools_fall_back_to_their_own_defaults_o")}
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
