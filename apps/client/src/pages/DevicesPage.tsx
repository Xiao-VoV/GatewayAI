import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { useClient } from '../auth/ClientProvider'
import { Badge, PageHead } from '../components/ui'

export function DevicesPage() {
  const { snapshot, session, forget, syncing, reload } = useClient()
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
        title="我的设备"
        desc="本机设备身份与虚拟 Key；解绑/禁用由管理员在管理台执行。"
        actions={
          <button className="btn" type="button" onClick={() => void reload()} disabled={syncing}>
            {syncing ? '同步中…' : '刷新'}
          </button>
        }
      />

      <div className="grid cols-2">
        <div className="card">
          <div className="row between">
            <h3>设备</h3>
            <Badge tone={snapshot.device.status === 'active' ? 'ok' : 'bad'}>{snapshot.device.status}</Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>名称</dt>
            <dd>{snapshot.device.name}</dd>
            <dt>平台</dt>
            <dd>{snapshot.device.platform}</dd>
            <dt>客户端版本</dt>
            <dd>{snapshot.device.client_version}</dd>
            <dt>最近在线</dt>
            <dd>{snapshot.device.last_seen_at ? new Date(snapshot.device.last_seen_at).toLocaleString('zh-CN') : '—'}</dd>
            <dt>device_id</dt>
            <dd>
              <code className="kbd">{snapshot.device.id}</code>
            </dd>
          </dl>
        </div>

        <div className="card">
          <div className="row between">
            <h3>虚拟 Key</h3>
            <Badge tone={snapshot.key.status === 'active' ? 'ok' : 'bad'}>{snapshot.key.status}</Badge>
          </div>
          <dl className="kv" style={{ marginTop: 10 }}>
            <dt>Key 前缀</dt>
            <dd>
              <code className="kbd">{snapshot.key.key_prefix}</code>
            </dd>
            <dt>过期</dt>
            <dd>{snapshot.key.expires_at ? new Date(snapshot.key.expires_at).toLocaleString('zh-CN') : '永不过期'}</dd>
            <dt>模型白名单</dt>
            <dd>{snapshot.key.allowed_models.length === 0 ? '不限（仍受发布约束）' : snapshot.key.allowed_models.join(', ')}</dd>
            <dt>账号</dt>
            <dd>
              {snapshot.user.display_name || snapshot.user.email} · {snapshot.user.email}
            </dd>
          </dl>
        </div>
      </div>

      <div className="card">
        <h3>连接</h3>
        <dl className="kv" style={{ marginTop: 10 }}>
          <dt>网关地址</dt>
          <dd>{session?.serverUrl}</dd>
          <dt>Token</dt>
          <dd>短期 Bearer，自动刷新；不展示明文</dd>
          <dt>设备密钥</dt>
          <dd>仅当前会话内存/系统钥匙串（Tauri 切片）；不落明文磁盘</dd>
        </dl>
      </div>

      <div className="card">
        <h3>解绑本机</h3>
        <p className="sub" style={{ marginTop: 4 }}>
          清除本机配对后需管理员重新生成配对码才能接入；管理员也可在管理台解绑设备。
        </p>
        {confirmForget ? (
          <div className="row" style={{ marginTop: 10 }}>
            <span className="hint">确定清除本机配对？</span>
            <button className="btn danger" type="button" onClick={handleForget}>
              确认清除
            </button>
            <button className="btn" type="button" onClick={() => setConfirmForget(false)}>
              取消
            </button>
          </div>
        ) : (
          <button className="btn danger" type="button" onClick={() => setConfirmForget(true)} style={{ marginTop: 10 }}>
            清除本机配对
          </button>
        )}
      </div>
    </>
  )
}
