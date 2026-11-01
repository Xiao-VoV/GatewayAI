import { useState } from 'react'
import type { FormEvent } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { ApiError } from '../api/client'
import { useAuth } from '../auth/AuthProvider'
import { useI18n } from '../i18n/I18nProvider'
import { LocaleSwitch } from '../i18n/LocaleSwitch'

interface LoginState {
  email?: string
  created?: boolean
}

function safeFrom(search: string): string {
  const value = new URLSearchParams(search).get('from')
  return value && value.startsWith('/') && !value.startsWith('//') ? value : '/'
}

export function LoginPage() {
  const auth = useAuth()
  const navigate = useNavigate()
  const location = useLocation()
  const { tr } = useI18n()
  const state = (location.state ?? null) as LoginState | null
  const [email, setEmail] = useState(state?.email ?? '')
  const [password, setPassword] = useState('')
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [submitting, setSubmitting] = useState(false)

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFormError(null)
    setFieldErrors({})
    if (!email.trim() || !password) {
      setFormError(tr("enter_your_email_and_password"))
      return
    }
    setSubmitting(true)
    try {
      await auth.login(email.trim(), password)
      navigate(safeFrom(location.search), { replace: true })
    } catch (error) {
      if (error instanceof ApiError) {
        setFieldErrors(error.errors)
        if (error.status === 429) {
          const wait = error.retryAfterSeconds
            ? tr("try_again_in_seconds", { retryAfterSeconds: error.retryAfterSeconds })
            : tr("try_again_later")
          setFormError(`${tr("too_many_sign_in_attempts")}${wait}`)
        } else if (error.status === 401) {
          setFormError(tr("incorrect_email_or_password"))
        } else if (error.status === 403) {
          setFormError(error.detail)
        } else {
          setFormError(error.detail)
        }
      } else {
        setFormError(tr("sign_in_failed_try_again"))
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
        <h1>Orveil Console</h1>
        <p>{tr("enterprise_ai_gateway_administrator_sign_in")}</p>
        {state?.created && (
          <div className="note" style={{ marginBottom: 14 }}>
            {tr("administrator_created_please_sign_in")}
          </div>
        )}
        <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate>
          <div className="field">
            <label htmlFor="login-email">{tr("email")}</label>
            <input
              id="login-email"
              className="input"
              type="email"
              autoComplete="username"
              autoFocus
              value={email}
              onChange={(event) => setEmail(event.target.value)}
            />
            {fieldErrors.email && <span className="field-error">{fieldErrors.email}</span>}
          </div>
          <div className="field">
            <label htmlFor="login-password">{tr("password")}</label>
            <input
              id="login-password"
              className="input"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            {fieldErrors.password && <span className="field-error">{fieldErrors.password}</span>}
          </div>
          {formError && (
            <div className="form-error" role="alert">
              {formError}
            </div>
          )}
          <button className="btn primary" type="submit" disabled={submitting}>
            {submitting ? tr("signing_in") : tr("sign_in")}
          </button>
          <span className="hint">
            {tr("cookie_session_csrf_failed_sign_ins_never_reveal_whether")}
          </span>
        </form>
      </div>
    </div>
  )
}
