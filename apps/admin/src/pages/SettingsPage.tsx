import { useEffect, useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import { listProviders } from '../api/providersApi'
import { getSettings, patchSettings } from '../api/settings'
import type { Settings } from '../api/settings'
import type { Provider } from '../api/providers'
import { useAsync } from '../api/useAsync'
import { PageHead } from '../components/ui'
import { ErrorPanel, LoadingPanel } from '../components/ScreenState'

export function SettingsPage() {
  const { data, loading, error, reload } = useAsync(
    () => Promise.all([getSettings(), listProviders({ limit: 200 })]).then(([settings, providers]) => ({
      settings,
      providers: providers.items,
    })),
    [],
  )

  const [form, setForm] = useState<Settings | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [savedAt, setSavedAt] = useState<string | null>(null)

  useEffect(() => {
    if (data) setForm(data.settings)
  }, [data])

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!form) return
    setSaving(true)
    setSaveError(null)
    setFieldErrors({})
    setSavedAt(null)
    try {
      const updated = await patchSettings({
        default_provider_id: form.default_provider_id,
        request_log_retention_days: form.request_log_retention_days,
        audit_retention_days: form.audit_retention_days,
        upstream_timeout_secs: form.upstream_timeout_secs,
      })
      setForm(updated)
      setSavedAt(new Date().toLocaleTimeString('zh-CN'))
    } catch (err) {
      if (err instanceof ApiError) {
        setFieldErrors(err.errors)
        setSaveError(err.detail)
      } else {
        setSaveError('保存设置失败')
      }
    } finally {
      setSaving(false)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead title="设置" desc="运行期默认供应商、日志/审计保留与上游超时。" />
        <LoadingPanel title="正在读取设置…" />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead title="设置" desc="运行期默认供应商、日志/审计保留与上游超时。" />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!form) return null

  return (
    <>
      <PageHead
        title="设置"
        desc="运行期默认供应商、日志/审计保留与上游超时；修改写入审计。"
        actions={
          <button className="btn" type="button" onClick={reload} disabled={saving}>
            重新读取
          </button>
        }
      />

      {saveError && (
        <div className="form-error" role="alert">
          {saveError}
        </div>
      )}
      {savedAt && <div className="note" role="status">已保存 · {savedAt}</div>}

      <form className="card grid" onSubmit={(event) => void handleSave(event)} noValidate>
        <div className="field">
          <label htmlFor="settings-default-provider">默认供应商</label>
          <select
            id="settings-default-provider"
            className="select"
            value={form.default_provider_id ?? ''}
            onChange={(event) =>
              setForm({ ...form, default_provider_id: event.target.value || null })
            }
          >
            <option value="">未设置</option>
            {data?.providers.map((provider: Provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.name} · {provider.status}
              </option>
            ))}
          </select>
          {fieldErrors.default_provider_id && (
            <span className="field-error">{fieldErrors.default_provider_id}</span>
          )}
          <span className="hint">默认供应商用于无显式供应商的部署/测试；可清除。</span>
        </div>

        <div className="grid cols-3">
          <div className="field">
            <label htmlFor="settings-request-retention">请求日志保留（天）</label>
            <input
              id="settings-request-retention"
              className="input"
              type="number"
              min={1}
              max={365}
              value={form.request_log_retention_days}
              onChange={(event) =>
                setForm({ ...form, request_log_retention_days: Number(event.target.value) })
              }
            />
            {fieldErrors.request_log_retention_days && (
              <span className="field-error">{fieldErrors.request_log_retention_days}</span>
            )}
            <span className="hint">1–365</span>
          </div>
          <div className="field">
            <label htmlFor="settings-audit-retention">审计保留（天）</label>
            <input
              id="settings-audit-retention"
              className="input"
              type="number"
              min={180}
              max={3650}
              value={form.audit_retention_days}
              onChange={(event) => setForm({ ...form, audit_retention_days: Number(event.target.value) })}
            />
            {fieldErrors.audit_retention_days && (
              <span className="field-error">{fieldErrors.audit_retention_days}</span>
            )}
            <span className="hint">180–3650（合规下限）</span>
          </div>
          <div className="field">
            <label htmlFor="settings-upstream-timeout">上游超时（秒）</label>
            <input
              id="settings-upstream-timeout"
              className="input"
              type="number"
              min={5}
              max={600}
              value={form.upstream_timeout_secs}
              onChange={(event) => setForm({ ...form, upstream_timeout_secs: Number(event.target.value) })}
            />
            {fieldErrors.upstream_timeout_secs && (
              <span className="field-error">{fieldErrors.upstream_timeout_secs}</span>
            )}
            <span className="hint">5–600 秒</span>
          </div>
        </div>

        <div className="row">
          <button className="btn primary" type="submit" disabled={saving}>
            {saving ? '保存中…' : '保存设置'}
          </button>
          <span className="hint">
            更新时间 {new Date(form.updated_at).toLocaleString('zh-CN')}
          </span>
        </div>
      </form>

      <div className="card">
        <h3>说明</h3>
        <p className="sub" style={{ marginTop: 6 }}>
          清理任务按保留天数删除旧 request_logs / audit_logs；上游超时作用于代理上游请求。设置变更会写审计。
        </p>
      </div>
    </>
  )
}
