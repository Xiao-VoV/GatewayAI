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
    if (!email.trim()) errors.email = tr('请输入管理员邮箱', 'Enter the administrator email')
    if (password.length < 10) errors.password = tr('密码至少 10 个字符', 'Password must be at least 10 characters')
    if (password !== confirm) errors.confirm = tr('两次输入的密码不一致', 'Passwords do not match')
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
        setFormError(tr('创建管理员失败，请重试', 'Failed to create administrator. Try again.'))
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
        <h1>{tr('初始化 Orveil Console', 'Set up Orveil Console')}</h1>
        <p>
          {tr(
            '第一次启动，创建管理员账号。创建后使用该账号登录。',
            'First launch: create the administrator account, then sign in with it.',
          )}
        </p>
        <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate>
          <div className="field">
            <label htmlFor="setup-email">{tr('邮箱', 'Email')}</label>
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
            <label htmlFor="setup-name">{tr('显示名（可选）', 'Display name (optional)')}</label>
            <input
              id="setup-name"
              className="input"
              autoComplete="name"
              placeholder={tr('平台管理员', 'Platform administrator')}
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
            />
            {fieldErrors.display_name && <span className="field-error">{fieldErrors.display_name}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-password">{tr('密码', 'Password')}</label>
            <input
              id="setup-password"
              className="input"
              type="password"
              autoComplete="new-password"
              placeholder={tr('至少 10 个字符', 'At least 10 characters')}
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            {fieldErrors.password && <span className="field-error">{fieldErrors.password}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-confirm">{tr('确认密码', 'Confirm password')}</label>
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
                {tr(
                  '服务端配置了 GW_SETUP_TOKEN，需要提供相同值',
                  'GW_SETUP_TOKEN is configured on the server; enter the same value here.',
                )}
              </span>
            </div>
          )}
          {formError && (
            <div className="form-error" role="alert">
              {formError}
            </div>
          )}
          <button className="btn primary" type="submit" disabled={submitting}>
            {submitting ? tr('创建中…', 'Creating…') : tr('创建管理员', 'Create administrator')}
          </button>
          <span className="hint">
            {tr('已有管理员后，此页面不可再次创建。', 'Once an administrator exists, this page can no longer create one.')}
          </span>
        </form>
      </div>
    </div>
  )
}
