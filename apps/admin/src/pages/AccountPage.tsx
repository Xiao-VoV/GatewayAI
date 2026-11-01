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
      setFieldErrors({ new_password: tr('新密码至少 10 个字符', 'New password must be at least 10 characters') })
      return
    }
    if (newPassword !== confirm) {
      setFieldErrors({ confirm: tr('两次输入的新密码不一致', 'New passwords do not match') })
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
        setFormError(tr('修改密码失败', 'Failed to change password'))
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
        title={tr('账号与会话', 'Account & session')}
        desc={tr('当前管理员、修改密码与退出登录。', 'Current administrator, password change and sign-out.')}
      />

      <div className="grid cols-2">
        <div className="card">
          <h3>{tr('当前管理员', 'Current administrator')}</h3>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{tr('显示名', 'Display name')}</dt>
            <dd>{session.user.display_name || '—'}</dd>
            <dt>{tr('邮箱', 'Email')}</dt>
            <dd>{session.user.email}</dd>
            <dt>{tr('状态', 'Status')}</dt>
            <dd>
              <Badge tone={session.user.status === 'active' ? 'ok' : 'bad'}>{session.user.status}</Badge>
            </dd>
            <dt>{tr('会话过期', 'Session expires')}</dt>
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
            {loggingOut ? tr('退出中…', 'Signing out…') : tr('退出登录', 'Sign out')}
          </button>
        </div>

        <div className="card">
          <h3>{tr('修改密码', 'Change password')}</h3>
          <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate style={{ marginTop: 10 }}>
            <div className="field">
              <label htmlFor="account-current">{tr('当前密码', 'Current password')}</label>
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
              <label htmlFor="account-new">{tr('新密码', 'New password')}</label>
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
              <label htmlFor="account-confirm">{tr('确认新密码', 'Confirm new password')}</label>
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
                {tr(
                  '密码已更新；其他会话已失效，当前会话保留。',
                  'Password updated. Other sessions were revoked; this session remains active.',
                )}
              </div>
            )}
            <button className="btn primary" type="submit" disabled={saving}>
              {saving ? tr('保存中…', 'Saving…') : tr('更新密码', 'Update password')}
            </button>
          </form>
        </div>
      </div>

      <div className="card">
        <h3>{tr('安全说明', 'Security notes')}</h3>
        <p className="sub" style={{ marginTop: 6 }}>
          {tr(
            'Session Cookie 为 HttpOnly；CSRF token 仅保存在内存，刷新后从 /auth/me 重新获取。修改密码会吊销其他会话。',
            'The session cookie is HttpOnly; the CSRF token is kept in memory only and re-fetched from /auth/me after a refresh. Changing the password revokes other sessions.',
          )}
        </p>
      </div>
    </>
  )
}
