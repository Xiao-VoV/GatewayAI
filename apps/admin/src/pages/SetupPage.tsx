import { useState } from 'react'
import type { FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ApiError } from '../api/client'
import { useAuth } from '../auth/AuthProvider'
import { useI18n } from '../i18n/I18nProvider'
import { LocaleSwitch } from '../i18n/LocaleSwitch'

export function SetupPage() {
  const auth = useAuth()
  const navigate = useNavigate()
  const { tr } = useI18n()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [displayName, setDisplayName] = useState('')
  const [setupToken, setSetupToken] = useState('')
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFormError(null)
    const errors: Record<string, string> = {}
    if (!email.trim()) errors.email = tr("enter_the_administrator_email")
    if (password.length < 10) errors.password = tr("password_must_be_at_least_10_characters")
    if (password !== confirm) errors.confirm = tr("passwords_do_not_match")
    setFieldErrors(errors)
    if (Object.keys(errors).length > 0) return

    setSubmitting(true)
    try {
      await auth.setupAdmin({
        email: email.trim(),
        password,
        displayName: displayName.trim(),
        setupToken: setupToken.trim() || undefined,
      })
      navigate('/login', { replace: true, state: { email: email.trim(), created: true } })
    } catch (error) {
      if (error instanceof ApiError) {
        setFieldErrors(error.errors)
        setFormError(error.detail)
      } else {
        setFormError(tr("failed_to_create_administrator_try_again"))
      }
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="login-wrap">
      <div className="login-card">
        <div className="login-locale"><LocaleSwitch /></div>
        <img className="login-logo" src="/logo.svg" alt="Orveil" />
        <h1>{tr("set_up_orveil_console")}</h1>
        <p>
          {tr("first_launch_create_the_administrator_account_then_sign")}
        </p>
        <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate>
          <div className="field">
            <label htmlFor="setup-email">{tr("email")}</label>
            <input
              id="setup-email"
              className="input"
              type="email"
              autoComplete="username"
              placeholder="admin@corp.example.com"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
            />
            {fieldErrors.email && <span className="field-error">{fieldErrors.email}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-name">{tr("display_name_optional")}</label>
            <input
              id="setup-name"
              className="input"
              autoComplete="name"
              placeholder={tr("platform_administrator")}
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
            />
            {fieldErrors.display_name && <span className="field-error">{fieldErrors.display_name}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-password">{tr("password")}</label>
            <input
              id="setup-password"
              className="input"
              type="password"
              autoComplete="new-password"
              placeholder={tr("at_least_10_characters")}
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            {fieldErrors.password && <span className="field-error">{fieldErrors.password}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-confirm">{tr("confirm_password")}</label>
            <input
              id="setup-confirm"
              className="input"
              type="password"
              autoComplete="new-password"
              value={confirm}
              onChange={(event) => setConfirm(event.target.value)}
            />
            {fieldErrors.confirm && <span className="field-error">{fieldErrors.confirm}</span>}
          </div>
          {auth.setupTokenRequired && (
            <div className="field">
              <label htmlFor="setup-token">Setup Token</label>
              <input
                id="setup-token"
                className="input"
                type="password"
                autoComplete="off"
                value={setupToken}
                onChange={(event) => setSetupToken(event.target.value)}
              />
              <span className="hint">
                {tr("gw_setup_token_is_configured_on_the_server_enter_the_sam")}
              </span>
            </div>
          )}
          {formError && (
            <div className="form-error" role="alert">
              {formError}
            </div>
          )}
          <button className="btn primary" type="submit" disabled={submitting}>
            {submitting ? tr("creating") : tr("create_administrator")}
          </button>
          <span className="hint">
            {tr("once_an_administrator_exists_this_page_can_no_longer_cre")}
          </span>
        </form>
      </div>
    </div>
  )
}
