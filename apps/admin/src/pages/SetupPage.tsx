import { useState } from 'react'
import type { FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ApiError } from '../api/client'
import { useAuth } from '../auth/AuthProvider'

export function SetupPage() {
  const auth = useAuth()
  const navigate = useNavigate()
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
    if (!email.trim()) errors.email = '请输入管理员邮箱'
    if (password.length < 10) errors.password = '密码至少 10 个字符'
    if (password !== confirm) errors.confirm = '两次输入的密码不一致'
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
        setFormError('创建管理员失败，请重试')
      }
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="login-wrap">
      <div className="login-card">
        <h1>初始化 gw admin</h1>
        <p>第一次启动，创建管理员账号。创建后使用该账号登录。</p>
        <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate>
          <div className="field">
            <label htmlFor="setup-email">邮箱</label>
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
            <label htmlFor="setup-name">显示名（可选）</label>
            <input
              id="setup-name"
              className="input"
              autoComplete="name"
              placeholder="平台管理员"
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
            />
            {fieldErrors.display_name && <span className="field-error">{fieldErrors.display_name}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-password">密码</label>
            <input
              id="setup-password"
              className="input"
              type="password"
              autoComplete="new-password"
              placeholder="至少 10 个字符"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
            {fieldErrors.password && <span className="field-error">{fieldErrors.password}</span>}
          </div>
          <div className="field">
            <label htmlFor="setup-confirm">确认密码</label>
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
              <span className="hint">服务端配置了 GW_SETUP_TOKEN，需要提供相同值</span>
            </div>
          )}
          {formError && (
            <div className="form-error" role="alert">
              {formError}
            </div>
          )}
          <button className="btn primary" type="submit" disabled={submitting}>
            {submitting ? '创建中…' : '创建管理员'}
          </button>
          <span className="hint">已有管理员后，此页面不可再次创建。</span>
        </form>
      </div>
    </div>
  )
}
