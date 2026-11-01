import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useClient } from '../auth/ClientProvider'
import { Badge, PageHead } from '../components/ui'
import { useUiPreferences } from '../ui/preferences'

export function DevicesPage() {
  const { snapshot, session, forget, syncing, reload } = useClient()
  const { locale } = useUiPreferences()
  const en = locale === 'en-US'
  const navigate = useNavigate()
  const [confirmForget, setConfirmForget] = useState(false)
  if (!snapshot) return null

  function handleForget() {
    forget()
    navigate('/welcome', { replace: true })
  }

  return (
    <>
      <PageHead
        title={en ? 'My Device' : '我的设备'}
        desc={en ? 'This device identity and virtual key. Unbinding or disabling is managed by administrators.' : '本机设备身份与虚拟 Key；解绑/禁用由管理员在管理台执行。'}
        actions={
          <button className="btn" type="button" onClick={() => void reload()} disabled={syncing}>
            {syncing ? (en ? 'Syncing…' : '同步中…') : en ? 'Refresh' : '刷新'}
          </button>
        }
      />

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>{en ? 'Device' : '设备'}</h3>
            <Badge tone={snapshot.device.status === 'active' ? 'ok' : 'bad'}>{snapshot.device.status}</Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{en ? 'Name' : '名称'}</dt>
            <dd>{snapshot.device.name}</dd>
            <dt>{en ? 'Platform' : '平台'}</dt>
            <dd>{snapshot.device.platform}</dd>
            <dt>{en ? 'Client version' : '客户端版本'}</dt>
            <dd>{snapshot.device.client_version}</dd>
            <dt>{en ? 'Last seen' : '最近在线'}</dt>
            <dd>{snapshot.device.last_seen_at ? new Date(snapshot.device.last_seen_at).toLocaleString(locale) : '—'}</dd>
            <dt>device_id</dt>
            <dd>
              <code className="kbd">{snapshot.device.id}</code>
            </dd>
          </dl>
        </div>

        <div className="card">
          <div className="row between">
            <h3>{en ? 'Virtual key' : '虚拟 Key'}</h3>
            <Badge tone={snapshot.key.status === 'active' ? 'ok' : 'bad'}>{snapshot.key.status}</Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>{en ? 'Key prefix' : 'Key 前缀'}</dt>
            <dd>
              <code className="kbd">{snapshot.key.key_prefix}</code>
            </dd>
            <dt>{en ? 'Expires' : '过期'}</dt>
            <dd>{snapshot.key.expires_at ? new Date(snapshot.key.expires_at).toLocaleString(locale) : en ? 'No expiry' : '永不过期'}</dd>
            <dt>{en ? 'Allowed models' : '模型白名单'}</dt>
            <dd>{snapshot.key.allowed_models.length === 0 ? (en ? 'Unrestricted (still subject to publications)' : '不限（仍受发布约束）') : snapshot.key.allowed_models.join(', ')}</dd>
            <dt>{en ? 'Account' : '账号'}</dt>
            <dd>
              {snapshot.user.display_name || snapshot.user.email} · {snapshot.user.email}
            </dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <h3>{en ? 'Connection' : '连接'}</h3>
        <dl className="kv" style={{ marginTop: 10 }}>
          <dt>{en ? 'Gateway URL' : '网关地址'}</dt>
          <dd>{session?.serverUrl}</dd>
          <dt>Token</dt>
          <dd>{en ? 'Short-lived bearer token, refreshed automatically; never shown in plaintext.' : '短期 Bearer，自动刷新；不展示明文'}</dd>
          <dt>{en ? 'Device secret' : '设备密钥'}</dt>
          <dd>{en ? 'Session memory / system keychain in the desktop app; never written as plaintext.' : '仅当前会话内存/系统钥匙串（Tauri 切片）；不落明文磁盘'}</dd>
        </dl>
      </div>

      <div className="card">
        <h3>{en ? 'Unbind this device' : '解绑本机'}</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          {en ? 'Removing this pairing requires a new pairing code from an administrator. Administrators can also unbind the device from the console.' : '清除本机配对后需管理员重新生成配对码才能接入；管理员也可在管理台解绑设备。'}
        </p>
        {confirmForget ? (
          <div className="row" style={{ marginTop: 10 }}>
            <span className="hint">{en ? 'Remove this pairing?' : '确定清除本机配对？'}</span>
            <button className="btn danger" type="button" onClick={handleForget}>
              {en ? 'Remove' : '确认清除'}
            </button>
            <button className="btn" type="button" onClick={() => setConfirmForget(false)}>
              {en ? 'Cancel' : '取消'}
            </button>
          </div>
        ) : (
          <button className="btn danger" type="button" onClick={() => setConfirmForget(true)} style={{ marginTop: 10 }}>
            {en ? 'Remove pairing' : '清除本机配对'}
          </button>
        )}
      </div>
    </>
  )
}
