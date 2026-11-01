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
import { useI18n } from '../i18n/I18nProvider'

export function SettingsPage() {
  const { tr } = useI18n()
  const summaryDesc = tr("default_provider_log_and_audit_retention_and_upstream_ti_2")
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
        setSaveError(tr("failed_to_save_settings"))
      }
    } finally {
      setSaving(false)
    }
  }

  if (loading && !data) {
    return (
      <>
        <PageHead title={tr("settings")} desc={summaryDesc} />
        <LoadingPanel title={tr("loading_settings")} />
      </>
    )
  }
  if (error) {
    return (
      <>
        <PageHead title={tr("settings")} desc={summaryDesc} />
        <ErrorPanel error={error} onRetry={reload} />
      </>
    )
  }
  if (!form) return null

  return (
    <>
      <PageHead
        title={tr("settings")}
        desc={tr("default_provider_log_and_audit_retention_and_upstream_ti")}
        actions={
          <button className="btn" type="button" onClick={reload} disabled={saving}>
            {tr("reload")}
          </button>
        }
      />

      {saveError && (
        <div className="form-error" role="alert">
          {saveError}
        </div>
      )}
      {savedAt && (
        <div className="note" role="status">
          {tr("saved")} · {savedAt}
        </div>
      )}

      <form className="card grid" onSubmit={(event) => void handleSave(event)} noValidate>
        <div className="field">
          <label htmlFor="settings-default-provider">{tr("default_provider")}</label>
          <select
            id="settings-default-provider"
            className="select"
            value={form.default_provider_id ?? ''}
            onChange={(event) =>
              setForm({ ...form, default_provider_id: event.target.value || null })
            }
          >
            <option value="">{tr("not_set")}</option>
            {data?.providers.map((provider: Provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.name} · {provider.status}
              </option>
            ))}
          </select>
          {fieldErrors.default_provider_id && (
            <span className="field-error">{fieldErrors.default_provider_id}</span>
          )}
          <span className="hint">
            {tr("used_for_deployments_and_tests_without_an_explicit_provi")}
          </span>
        </div>

        <div className="grid cols-3">
          <div className="field">
            <label htmlFor="settings-request-retention">{tr("request_log_retention_days")}</label>
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
            <label htmlFor="settings-audit-retention">{tr("audit_retention_days")}</label>
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
            <span className="hint">{tr("180_3650_compliance_minimum")}</span>
          </div>
          <div className="field">
            <label htmlFor="settings-upstream-timeout">{tr("upstream_timeout_seconds")}</label>
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
            <span className="hint">{tr("5_600_seconds")}</span>
          </div>
        </div>

        <div className="row">
          <button className="btn primary" type="submit" disabled={saving}>
            {saving ? tr("saving") : tr("save_settings")}
          </button>
          <span className="hint">
            {tr("last_updated")} {new Date(form.updated_at).toLocaleString('zh-CN')}
          </span>
        </div>
      </form>

      <div className="card">
        <h3>{tr("notes")}</h3>
        <p className="sub" style={{ marginTop: 6 }}>
          {tr("cleanup_jobs_delete_old_request_logs_and_audit_logs_per")}
        </p>
      </div>
    </>
  )
}
