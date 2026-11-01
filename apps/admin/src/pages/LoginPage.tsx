import { useState } from 'react'
import type { FormEvent } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { ApiError } from '../api/client'
import { useAuth } from '../auth/AuthProvider'

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
      setFormError('请输入邮箱和密码')
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
          const wait = error.retryAfterSeconds ? `，请 ${error.retryAfterSeconds} 秒后重试` : '，请稍后重试'
          setFormError(`登录尝试过于频繁${wait}`)
        } else if (error.status === 401) {
          setFormError('邮箱或密码不正确')
        } else if (error.status === 403) {
          setFormError(error.detail)
        } else {
          setFormError(error.detail)
        }
      } else {
        setFormError('登录失败，请重试')
      }
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <div className="login-wrap">
      <div className="login-card">
        <h1>gw admin</h1>
        <p>管理员登录</p>
        {state?.created && <div className="note" style={{ marginBottom: 14 }}>管理员已创建，请登录。</div>}
        <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate>
          <div className="field">
            <label htmlFor="login-email">邮箱</label>
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
            <label htmlFor="login-password">密码</label>
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
            {submitting ? '登录中…' : '登录'}
          </button>
          <span className="hint">Cookie Session + CSRF；登录失败不会透露账号是否存在。</span>
        </form>
      </div>
    </div>
  )
}
