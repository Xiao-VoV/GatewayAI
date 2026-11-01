import { useState } from 'react'
import type { FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { ApiError } from '../api/client'
import { createModelGroup, listModelGroups } from '../api/catalogApi'
import type { ModelGroupCreateInput, ModelGroupSummary } from '../api/catalog'
import { useAsync } from '../api/useAsync'
import { Badge, PageHead } from '../components/ui'
import { EmptyPanel, ErrorPanel, LoadingPanel } from '../components/ScreenState'
import { Modal } from '../components/Modal'

interface GroupDraft {
  client_model_name: string
  display_name: string
  canonical: string
  description: string
  capabilities: string
  context_window: string
  max_output: string
  protocols: string
}

const emptyDraft: GroupDraft = {
  client_model_name: '',
  display_name: '',
  canonical: '',
  description: '',
  capabilities: 'chat, stream',
  context_window: '',
  max_output: '',
  protocols: 'openai_chat',
}

function toInput(draft: GroupDraft): ModelGroupCreateInput {
  return {
    client_model_name: draft.client_model_name.trim(),
    display_name: draft.display_name.trim() || undefined,
    canonical: draft.canonical.trim(),
    description: draft.description.trim(),
    capabilities: draft.capabilities
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean),
    context_window: draft.context_window.trim() ? Number(draft.context_window) : null,
    max_output: draft.max_output.trim() ? Number(draft.max_output) : null,
    protocols: draft.protocols
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean),
    status: 'draft',
  }
}

function StatusBadge({ status }: { status: ModelGroupSummary['status'] }) {
  if (status === 'active') return <Badge tone="ok">启用</Badge>
  if (status === 'draft') return <Badge tone="warn">草稿</Badge>
  return <Badge>已归档</Badge>
}

export function ModelsPage() {
  const navigate = useNavigate()
  const [query, setQuery] = useState('')
  const [status, setStatus] = useState('')
  const [appliedQuery, setAppliedQuery] = useState('')
  const [modalOpen, setModalOpen] = useState(false)
  const [draft, setDraft] = useState<GroupDraft>(emptyDraft)
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [formError, setFormError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)

  const { data, loading, error, reload } = useAsync(
    () => listModelGroups({ q: appliedQuery || undefined, status: status || undefined, limit: 100 }),
    [appliedQuery, status],
  )

  function openCreate() {
    setDraft(emptyDraft)
    setFieldErrors({})
    setFormError(null)
    setModalOpen(true)
  }

  async function handleCreate(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setFormError(null)
    setSaving(true)
    try {
      const group = await createModelGroup(toInput(draft))
      setModalOpen(false)
      navigate(`/models/${group.id}`)
    } catch (err) {
      if (err instanceof ApiError) {
        setFieldErrors(err.errors)
        setFormError(err.detail)
      } else {
        setFormError('创建模型组失败，请重试')
      }
    } finally {
      setSaving(false)
    }
  }

  function submitSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setAppliedQuery(query.trim())
  }

  const refresh = (
    <button className="btn" type="button" onClick={reload} disabled={loading}>
      {loading ? '刷新中…' : '刷新'}
    </button>
  )

  return (
    <>
      <PageHead
        title="模型目录"
        desc="对外模型组与部署；草稿添加部署后再激活、发布。"
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              新建模型组
            </button>
          </>
        }
      />

      <div className="card">
        <form className="toolbar" onSubmit={submitSearch}>
          <div className="field" style={{ width: 260 }}>
            <label htmlFor="group-query">搜索</label>
            <input
              id="group-query"
              className="input"
              placeholder="对外模型名 / 显示名 / canonical"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <div className="field" style={{ width: 150 }}>
            <label htmlFor="group-status">状态</label>
            <select
              id="group-status"
              className="select"
              value={status}
              onChange={(event) => setStatus(event.target.value)}
            >
              <option value="">全部</option>
              <option value="draft">草稿</option>
              <option value="active">启用</option>
              <option value="archived">已归档</option>
            </select>
          </div>
          <button className="btn" type="submit" style={{ marginTop: 18 }}>
            查询
          </button>
        </form>
      </div>

      {loading && !data ? (
        <LoadingPanel title="正在读取模型目录…" />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={appliedQuery || status ? '没有匹配的模型组' : '还没有模型组'}
          desc="创建模型组（草稿）→ 在详情里添加上游部署 → 激活 → 发布给用户/组。"
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              新建模型组
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>对外模型名</th>
                <th>显示名</th>
                <th>Canonical</th>
                <th>状态</th>
                <th>部署</th>
                <th>发布</th>
                <th>工具默认</th>
                <th>告警</th>
              </tr>
            </thead>
            <tbody>
              {data.items.map((group) => (
                <tr key={group.id} className="clickable" onClick={() => navigate(`/models/${group.id}`)}>
                  <td>
                    <strong>{group.client_model_name}</strong>
                    <div className="hint">{group.id.slice(0, 8)}</div>
                  </td>
                  <td>{group.display_name || '—'}</td>
                  <td>{group.canonical}</td>
                  <td>
                    <StatusBadge status={group.status} />
                  </td>
                  <td className="num">
                    {group.enabled_deployment_count}/{group.deployment_count}
                  </td>
                  <td>
                    {group.publication_status ? (
                      <Badge tone={group.publication_status === 'active' ? 'ok' : 'warn'}>
                        {group.publication_status}
                      </Badge>
                    ) : (
                      <span className="hint">未发布</span>
                    )}
                  </td>
                  <td className="num">{group.tool_default_count}</td>
                  <td>
                    {group.warnings.length === 0 ? (
                      <span className="hint">—</span>
                    ) : (
                      <Badge tone="warn">{group.warnings.join(', ')}</Badge>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="hint" style={{ padding: '8px 12px' }}>
            共 {data.total} 个模型组
          </div>
        </div>
      )}

      {modalOpen && (
        <Modal
          title="新建模型组"
          sub="先创建草稿；在详情添加部署后激活/发布"
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={saving}>
                取消
              </button>
              <button className="btn primary" type="submit" form="create-group-form" disabled={saving}>
                {saving ? '创建中…' : '创建草稿'}
              </button>
            </>
          }
        >
          <form id="create-group-form" className="grid" onSubmit={(event) => void handleCreate(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-client-name">对外模型名</label>
                <input
                  id="group-client-name"
                  className="input"
                  placeholder="deepseek-chat"
                  value={draft.client_model_name}
                  onChange={(event) => setDraft({ ...draft, client_model_name: event.target.value })}
                />
                {fieldErrors.client_model_name && (
                  <span className="field-error">{fieldErrors.client_model_name}</span>
                )}
              </div>
              <div className="field">
                <label htmlFor="group-display-name">显示名</label>
                <input
                  id="group-display-name"
                  className="input"
                  placeholder="例如 DeepSeek Chat"
                  value={draft.display_name}
                  onChange={(event) => setDraft({ ...draft, display_name: event.target.value })}
                />
              </div>
            </div>
            <div className="field">
              <label htmlFor="group-canonical">Canonical</label>
              <input
                id="group-canonical"
                className="input"
                placeholder="必须与部署上游模型的 canonical 一致"
                value={draft.canonical}
                onChange={(event) => setDraft({ ...draft, canonical: event.target.value })}
              />
              {fieldErrors.canonical && <span className="field-error">{fieldErrors.canonical}</span>}
            </div>
            <div className="field">
              <label htmlFor="group-description">描述</label>
              <input
                id="group-description"
                className="input"
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-capabilities">能力标签（逗号分隔）</label>
                <input
                  id="group-capabilities"
                  className="input"
                  value={draft.capabilities}
                  onChange={(event) => setDraft({ ...draft, capabilities: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="group-protocols">协议（逗号分隔）</label>
                <input
                  id="group-protocols"
                  className="input"
                  value={draft.protocols}
                  onChange={(event) => setDraft({ ...draft, protocols: event.target.value })}
                />
              </div>
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-context">上下文窗口</label>
                <input
                  id="group-context"
                  className="input"
                  type="number"
                  value={draft.context_window}
                  onChange={(event) => setDraft({ ...draft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="group-max-output">最大输出</label>
                <input
                  id="group-max-output"
                  className="input"
                  type="number"
                  value={draft.max_output}
                  onChange={(event) => setDraft({ ...draft, max_output: event.target.value })}
                />
              </div>
            </div>
            {formError && (
              <div className="form-error" role="alert">
                {formError}
              </div>
            )}
          </form>
        </Modal>
      )}
    </>
  )
}
