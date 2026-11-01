import { useState } from 'react'
import { useClient } from '../auth/ClientProvider'
import { Badge, PageHead } from '../components/ui'
import { EmptyPanel } from '../components/ScreenState'

export function ModelsPage() {
  const { snapshot, syncing, reload } = useClient()
  const [query, setQuery] = useState('')
  if (!snapshot) return null

  const keyword = query.trim().toLowerCase()
  const models = snapshot.models.filter(
    (model) =>
      !keyword ||
      model.id.toLowerCase().includes(keyword) ||
      model.display_name.toLowerCase().includes(keyword) ||
      (model.upstream_model ?? '').toLowerCase().includes(keyword),
  )

  return (
    <>
      <PageHead
        title="模型"
        desc="本组已发布的模型；只展示可见且可调用的目录，不含价格/供应商。"
        actions={
          <button className="btn" type="button" onClick={() => void reload()} disabled={syncing}>
            {syncing ? '同步中…' : '刷新目录'}
          </button>
        }
      />

      <div className="card">
        <div className="filters">
          <input
            className="input"
            style={{ width: 280 }}
            placeholder="搜索模型名 / 上游模型"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
          <span className="hint">
            {models.length} 个模型 · catalog {snapshot.catalogVersion || '—'}
          </span>
        </div>
      </div>

      {models.length === 0 ? (
        <EmptyPanel
          title={keyword ? '没有匹配的模型' : '本组还没有已发布模型'}
          desc="模型由管理员发布到全部用户/用户组；联系管理员检查发布与可见状态。"
        />
      ) : (
        <div className="model-grid">
          {models.map((model) => (
            <div className="model-card" key={model.id}>
              <div className="row between">
                <h3>{model.display_name}</h3>
                <Badge tone="ok">可用</Badge>
              </div>
              <div className="mono">{model.id}</div>
              <div className="hint" style={{ marginTop: 4 }}>
                上游 {model.upstream_model ?? '—'}
              </div>
              <div className="meta">
                {model.capabilities.map((capability) => (
                  <span className="badge" key={capability}>
                    {capability}
                  </span>
                ))}
                {model.protocols.map((protocol) => (
                  <span className="badge accent" key={protocol}>
                    {protocol}
                  </span>
                ))}
              </div>
              <div className="hint" style={{ marginTop: 8 }}>
                上下文 {model.context_window ?? '—'} · 最大输出 {model.max_output ?? '—'}
              </div>
            </div>
          ))}
        </div>
      )}
    </>
  )
}
