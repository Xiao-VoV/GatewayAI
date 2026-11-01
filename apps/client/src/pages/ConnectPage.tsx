import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { defaultDeviceName, detectPlatform, useClient } from '../auth/ClientProvider'
import { ClientApiError, normalizeServerUrl } from '../api/clientApi'
import { OnboardingShell } from '../components/OnboardingShell'
import { useUiPreferences } from '../ui/preferences'

export function ConnectPage() {
  const client = useClient()
  const navigate = useNavigate()
  const { locale } = useUiPreferences()
  const en = locale === 'en-US'
  const [serverUrl, setServerUrl] = useState('http://127.0.0.1:8080')
  const [code, setCode] = useState('')
  const [deviceName, setDeviceName] = useState(() => defaultDeviceName(locale))
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setError(null)
    setFieldErrors({})
    const normalized = normalizeServerUrl(serverUrl)
    if (!/^https?:\/\/[^\s]+$/i.test(normalized)) {
      setFieldErrors({ server: en ? 'Enter an http(s) gateway URL' : '请输入 http(s) 企业网关地址' })
      return
    }
    if (!code.trim()) {
      setFieldErrors({ code: en ? 'Enter the pairing code' : '请输入配对码' })
      return
    }
    setBusy(true)
    try {
      await client.pair({ serverUrl: normalized, code: code.trim(), deviceName })
      navigate('/', { replace: true })
    } catch (err) {
      if (err instanceof ClientApiError) {
        setFieldErrors(err.errors)
        if (err.code === 'pairing_code_invalid') setError(en ? 'The pairing code is invalid, expired or already used. Ask an administrator for a new one.' : '配对码无效、已过期或已使用，请联系管理员重新生成。')
        else if (err.code === 'user_disabled') setError(en ? 'This account is disabled. Contact your administrator.' : '该员工账号已被禁用，请联系管理员。')
        else if (err.status === 429) setError(en ? `Too many attempts${err.retryAfterSeconds ? `, retry in ${err.retryAfterSeconds}s` : ', try again later'}` : `尝试过于频繁${err.retryAfterSeconds ? `，请 ${err.retryAfterSeconds} 秒后重试` : '，请稍后重试'}`)
        else if (err.code === 'network_error') setError(err.detail)
        else setError(err.detail)
      } else {
        setError(en ? 'Pairing failed. Try again.' : '配对失败，请重试')
      }
    } finally {
      setBusy(false)
    }
  }

  return (
    <OnboardingShell
      eyebrow={en ? 'DEVICE CONNECTION' : '设备连接'}
      title={en ? 'Connect to your gateway' : '连接企业网关'}
      description={
        en
          ? 'Register this device with a one-time pairing code. The local router will manage keys and device tokens for you.'
          : '使用管理员提供的一次性配对码注册本机。连接成功后，Key 和设备令牌由本地路由统一管理。'
      }
      points={
        en
          ? [
              { title: 'Short-lived credentials', desc: 'Device tokens last 15 minutes and refresh automatically.' },
              { title: 'Loopback router', desc: 'Listens on 127.0.0.1 only, never on the LAN.' },
              { title: 'Remove anytime', desc: 'Clear this device pairing safely in “My Device”.' },
            ]
          : [
              { title: '短期凭证', desc: '设备 token 15 分钟有效，自动刷新。' },
              { title: '本机路由', desc: '仅监听 127.0.0.1，不暴露到局域网。' },
              { title: '随时清除', desc: '本机配对可在“我的设备”中安全清除。' },
            ]
      }
    >
      <div className="onboarding-form-head">
        <span className="eyebrow">{en ? 'CONNECT GATEWAY' : '连接网关'}</span>
        <h2>{en ? 'Register this device' : '注册这台设备'}</h2>
        <p>{en ? 'Enter the gateway URL and pairing code. The device name appears in the admin console.' : '填写企业网关地址和配对码，设备名会显示在管理台。'}</p>
      </div>

      <form className="form-stack" onSubmit={(event) => void handleSubmit(event)} noValidate>
        <div className="field">
          <label htmlFor="server-url">{en ? 'Gateway URL' : '企业网关地址'}</label>
          <input
            id="server-url"
            className="input"
            placeholder="https://gw.corp.example.com"
            value={serverUrl}
            onChange={(event) => setServerUrl(event.target.value)}
          />
          {fieldErrors.server && <span className="field-error">{fieldErrors.server}</span>}
          <span className="hint">{en ? 'Local dev defaults to 127.0.0.1:8080; use your HTTPS gateway in production.' : '本地开发默认 127.0.0.1:8080；生产填企业网关 HTTPS 地址。'}</span>
        </div>
        <div className="field">
          <label htmlFor="pairing-code">{en ? 'Pairing code' : '配对码'}</label>
          <input
            id="pairing-code"
            className="input input-code"
            autoComplete="off"
            placeholder="7Q2M-9XKD"
            value={code}
            onChange={(event) => setCode(event.target.value)}
          />
          {fieldErrors.code && <span className="field-error">{fieldErrors.code}</span>}
        </div>
        <div className="field">
          <label htmlFor="device-name">{en ? 'Device name' : '设备名称'}</label>
          <input
            id="device-name"
            className="input"
            value={deviceName}
            onChange={(event) => setDeviceName(event.target.value)}
          />
          <span className="hint">{en ? `Platform detected: ${detectPlatform()}` : `平台自动识别：${detectPlatform()}`}</span>
        </div>
        {error && (
          <div className="form-error" role="alert">
            {error}
          </div>
        )}
        <button className="btn primary btn-block" type="submit" disabled={busy}>
          {busy ? (en ? 'Connecting…' : '正在连接…') : en ? 'Connect & register' : '连接并注册'}
        </button>
      </form>

      <div className="onboarding-back">
        <Link to="/welcome">{en ? 'Back' : '返回上一步'}</Link>
        <span>{en ? 'Credentials are stored in the system keychain' : '设备凭据保存在系统钥匙串'}</span>
      </div>
    </OnboardingShell>
  )
}
