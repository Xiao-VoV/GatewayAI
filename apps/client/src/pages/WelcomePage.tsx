import { Link } from 'react-router-dom'
import { CLIENT_VERSION } from '../auth/ClientProvider'

export function WelcomePage() {
  return (
    <div className="pairing-wrap">
      <div className="pairing-card">
        <div className="pairing-head">
          <span className="tool-icon" style={{ width: 36, height: 36 }}>
            gw
          </span>
          <div>
            <h2 style={{ margin: 0, fontSize: 18 }}>让 AI 工具接上企业网关</h2>
            <div className="hint">统一模型、用量与权限 · client v{CLIENT_VERSION}</div>
          </div>
        </div>
        <div className="pairing-body">
          <div className="grid cols-2">
            <section className="panel">
              <div className="panel-head">
                <h3>连接后</h3>
              </div>
              <div className="list-row">
                <span>设备注册并领取虚拟 Key</span>
              </div>
              <div className="list-row">
                <span>查看本组已发布的模型目录</span>
              </div>
              <div className="list-row">
                <span>模型与额度由企业统一管理</span>
              </div>
            </section>
            <section className="panel">
              <div className="panel-head">
                <h3>安全边界</h3>
              </div>
              <div className="list-row">
                <span>Key 只在配对时下发一次</span>
              </div>
              <div className="list-row">
                <span>设备 token 短期有效并自动刷新</span>
              </div>
              <div className="list-row">
                <span>不读取对话内容</span>
              </div>
            </section>
          </div>
          <div className="pairing-actions">
            <Link className="btn primary" to="/connect">
              连接企业网关
            </Link>
            <span className="hint" style={{ alignSelf: 'center' }}>
              需要管理员生成的一次性配对码
            </span>
          </div>
        </div>
      </div>
    </div>
  )
}
