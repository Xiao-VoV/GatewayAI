import { useState } from 'react'
import type { FormEvent } from 'react'
import { ApiError } from '../api/client'
import { changePassword } from '../api/auth'
import { useAuth } from '../auth/AuthProvider'
import { Badge, PageHead } from '../components/ui'
import { useNavigate } from 'react-router-dom'

export function AccountPage() {
  const auth = useAuth()
  const navigate = useNavigate()
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
      setFieldErrors({ new_password: '新密码至少 10 个字符' })
      return
    }
    if (newPassword !== confirm) {
      setFieldErrors({ confirm: '两次输入的新密码不一致' })
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
        setFormError('修改密码失败')
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
      <PageHead title="账号与会话" desc="当前管理员、修改密码与退出登录。" />

      <div className="grid cols-2">
        <div className="card">
          <h3>当前管理员</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>显示名</dt>
            <dd>{session.user.display_name || '—'}</dd>
            <dt>邮箱</dt>
            <dd>{session.user.email}</dd>
            <dt>状态</dt>
            <dd>
              <Badge tone={session.user.status === 'active' ? 'ok' : 'bad'}>{session.user.status}</Badge>
            </dd>
            <dt>会话过期</dt>
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
            {loggingOut ? '退出中…' : '退出登录'}
          </button>
        </div>

        <div className="card">
          <h3>修改密码</h3>
          <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate style={{ marginTop: 10 }}>
            <div className="field">
              <label htmlFor="account-current">当前密码</label>
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
              <label htmlFor="account-new">新密码</label>
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
              <label htmlFor="account-confirm">确认新密码</label>
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
            {success && <div className="note" role="status">密码已更新；其他会话已失效，当前会话保留。</div>}
            <button className="btn primary" type="submit" disabled={saving}>
              {saving ? '保存中…' : '更新密码'}
            </button>
          </form>
        </div>
      </div>

      <div className="card">
        <h3>安全说明</h3>
        <p className="sub" style={{ marginTop: 6 }}>
          Session Cookie 为 HttpOnly；CSRF token 仅保存在内存，刷新后从 /auth/me 重新获取。修改密码会吊销其他会话。
        </p>
      </div>
    </>
  )
}
