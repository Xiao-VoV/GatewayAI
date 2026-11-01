import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import { changePassword } from '../api/auth'
import { useAuth } from '../auth/AuthProvider'
import { Badge, PageHead } from '../components/ui'
import { useI18n } from '../i18n/I18nProvider'
import { useNavigate } from 'react-router-dom'

export function AccountPage() {
  const auth = useAuth()
  const navigate = useNavigate()
  const { tr } = useI18n()
  const session = auth.session
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [success, setSuccess] = useState(false)
  const [saving, setSaving] = useState(false)
  const [loggingOut, setLoggingOut] = useState(false)

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFormError(null)
    setFieldErrors({})
    setSuccess(false)
    if (newPassword.length < 10) {
      setFieldErrors({ new_password: tr("new_password_must_be_at_least_10_characters") })
      return
    }
    if (newPassword !== confirm) {
      setFieldErrors({ confirm: tr("new_passwords_do_not_match") })
      return
    }
    setSaving(true)
    try {
      await changePassword(currentPassword, newPassword)
      setSuccess(true)
      setCurrentPassword('')
      setNewPassword('')
      setConfirm('')
    } catch (err) {
      if (err instanceof ApiError) {
        setFieldErrors(err.errors)
        setFormError(err.detail)
      } else {
        setFormError(tr("failed_to_change_password"))
      }
    } finally {
      setSaving(false)
    }
  }

  async function handleLogout() {
    setLoggingOut(true)
    try {
      await auth.logout()
      navigate('/login', { replace: true })
    } finally {
      setLoggingOut(false)
    }
  }

  if (!session) return null

  return (
    <>
      <PageHead
        title={tr("account_session")}
        desc={tr("current_administrator_password_change_and_sign_out")}
      />

      <div className="grid cols-2">
        <div className="card">
          <h3>{tr("current_administrator")}</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{tr("display_name")}</dt>
            <dd>{session.user.display_name || '—'}</dd>
            <dt>{tr("email")}</dt>
            <dd>{session.user.email}</dd>
            <dt>{tr("status")}</dt>
            <dd>
              <Badge tone={session.user.status === 'active' ? 'ok' : 'bad'}>{session.user.status}</Badge>
            </dd>
            <dt>{tr("session_expires")}</dt>
            <dd>{new Date(session.expires_at).toLocaleString('zh-CN')}</dd>
            <dt>user_id</dt>
            <dd>
              <code className="kbd">{session.user.id}</code>
            </dd>
          </dl>
          <button
            className="btn danger"
            type="button"
            onClick={() => void handleLogout()}
            disabled={loggingOut}
            style={{ marginTop: 14 }}
          >
            {loggingOut ? tr("signing_out") : tr("sign_out_2")}
          </button>
        </div>

        <div className="card">
          <h3>{tr("change_password")}</h3>
          <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate style={{ marginTop: 10 }}>
            <div className="field">
              <label htmlFor="account-current">{tr("current_password")}</label>
              <input
                id="account-current"
                className="input"
                type="password"
                autoComplete="current-password"
                value={currentPassword}
                onChange={(event) => setCurrentPassword(event.target.value)}
              />
              {fieldErrors.current_password && <span className="field-error">{fieldErrors.current_password}</span>}
            </div>
            <div className="field">
              <label htmlFor="account-new">{tr("new_password")}</label>
              <input
                id="account-new"
                className="input"
                type="password"
                autoComplete="new-password"
                value={newPassword}
                onChange={(event) => setNewPassword(event.target.value)}
              />
              {fieldErrors.new_password && <span className="field-error">{fieldErrors.new_password}</span>}
            </div>
            <div className="field">
              <label htmlFor="account-confirm">{tr("confirm_new_password")}</label>
              <input
                id="account-confirm"
                className="input"
                type="password"
                autoComplete="new-password"
                value={confirm}
                onChange={(event) => setConfirm(event.target.value)}
              />
              {fieldErrors.confirm && <span className="field-error">{fieldErrors.confirm}</span>}
            </div>
            {formError && (
              <div className="form-error" role="alert">
                {formError}
              </div>
            )}
            {success && (
              <div className="note" role="status">
                {tr("password_updated_other_sessions_were_revoked_this_sessio")}
              </div>
            )}
            <button className="btn primary" type="submit" disabled={saving}>
              {saving ? tr("saving") : tr("update_password")}
            </button>
          </form>
        </div>
      </div>

      <div className="card">
        <h3>{tr("security_notes")}</h3>
        <p className="sub" style={{ marginTop: 6 }}>
          {tr("the_session_cookie_is_httponly_the_csrf_token_is_kept_in")}
        </p>
      </div>
    </>
  )
}
