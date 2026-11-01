import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { defaultDeviceName, detectPlatform, useClient } from '../auth/ClientProvider'
import { ClientApiError, normalizeServerUrl } from '../api/clientApi'

export function ConnectPage() {
  const client = useClient()
  const navigate = useNavigate()
  const [serverUrl, setServerUrl] = useState('http://127.0.0.1:8080')
  const [code, setCode] = useState('')
  const [deviceName, setDeviceName] = useState(defaultDeviceName())
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setError(null)
    setFieldErrors({})
    const normalized = normalizeServerUrl(serverUrl)
    if (!/^https?:\/\/[^\s]+$/i.test(normalized)) {
      setFieldErrors({ server: '请输入 http(s) 企业网关地址' })
      return
    }
    if (!code.trim()) {
      setFieldErrors({ code: '请输入配对码' })
      return
    }
    setBusy(true)
    try {
      await client.pair({ serverUrl: normalized, code: code.trim(), deviceName })
      navigate('/', { replace: true })
    } catch (err) {
      if (err instanceof ClientApiError) {
        setFieldErrors(err.errors)
        if (err.code === 'pairing_code_invalid') setError('配对码无效、已过期或已使用，请联系管理员重新生成。')
        else if (err.code === 'user_disabled') setError('该员工账号已被禁用，请联系管理员。')
        else if (err.status === 429) setError(`尝试过于频繁${err.retryAfterSeconds ? `，请 ${err.retryAfterSeconds} 秒后重试` : '，请稍后重试'}`)
        else if (err.code === 'network_error') setError(err.detail)
        else setError(err.detail)
      } else {
        setError('配对失败，请重试')
      }
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="pairing-wrap">
      <div className="pairing-card">
        <div className="pairing-head">
          <span className="tool-icon" style={{ width: 36, height: 36 }}>
            gw
          </span>
          <div>
            <h2 style={{ margin: 0, fontSize: 18 }}>连接企业网关</h2>
            <div className="hint">使用管理员提供的一次性配对码注册本机</div>
          </div>
        </div>
        <div className="pairing-body">
          <form className="grid" onSubmit={(event) => void handleSubmit(event)} noValidate>
            <div className="field">
              <label htmlFor="server-url">企业网关地址</label>
              <input
                id="server-url"
                className="input"
                placeholder="https://gw.corp.example.com"
                value={serverUrl}
                onChange={(event) => setServerUrl(event.target.value)}
              />
              {fieldErrors.server && <span className="field-error">{fieldErrors.server}</span>}
              <span className="hint">本地开发默认 127.0.0.1:8080；生产填企业网关 HTTPS 地址。</span>
            </div>
            <div className="field">
              <label htmlFor="pairing-code">配对码</label>
              <input
                id="pairing-code"
                className="input"
                autoComplete="off"
                placeholder="例如 7Q2M-9XKD"
                value={code}
                onChange={(event) => setCode(event.target.value)}
              />
              {fieldErrors.code && <span className="field-error">{fieldErrors.code}</span>}
            </div>
            <div className="field">
              <label htmlFor="device-name">设备名称</label>
              <input
                id="device-name"
                className="input"
                value={deviceName}
                onChange={(event) => setDeviceName(event.target.value)}
              />
              <span className="hint">平台自动识别：{detectPlatform()}</span>
            </div>
            {error && (
              <div className="form-error" role="alert">
                {error}
              </div>
            )}
            <div className="pairing-actions" style={{ marginTop: 4 }}>
              <button className="btn primary" type="submit" disabled={busy}>
                {busy ? '正在配对…' : '连接并注册'}
              </button>
              <Link className="btn" to="/welcome">
                返回
              </Link>
            </div>
            <p className="hint">
              配对成功后设备密钥与虚拟 Key 仅保存在当前会话（Tauri 版本将写入系统钥匙串）；刷新页面会重新校验 token。
            </p>
          </form>
        </div>
      </div>
    </div>
  )
}
