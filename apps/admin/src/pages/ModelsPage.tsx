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
import { useI18n } from '../i18n/I18nProvider'

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
  const { tr } = useI18n()
  if (status === 'active') return <Badge tone="ok">{tr('启用', 'Active')}</Badge>
  if (status === 'draft') return <Badge tone="warn">{tr('草稿', 'Draft')}</Badge>
  return <Badge>{tr('已归档', 'Archived')}</Badge>
}

export function ModelsPage() {
  const navigate = useNavigate()
  const { tr } = useI18n()
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
        setFormError(tr('创建模型组失败，请重试', 'Failed to create the model group. Please try again.'))
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
      {loading ? tr('刷新中…', 'Refreshing…') : tr('刷新', 'Refresh')}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr('模型目录', 'Model catalog')}
        desc={tr(
          '对外模型组与部署；草稿添加部署后再激活、发布。',
          'Client-facing model groups and deployments; add deployments to a draft, then activate and publish.',
        )}
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr('新建模型组', 'New model group')}
            </button>
          </>
        }
      />

      <div className="card">
        <form className="toolbar" onSubmit={submitSearch}>
          <div className="field" style={{ width: 260 }}>
            <label htmlFor="group-query">{tr('搜索', 'Search')}</label>
            <input
              id="group-query"
              className="input"
              placeholder={tr('对外模型名 / 显示名 / canonical', 'Client model / display name / canonical')}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <div className="field" style={{ width: 150 }}>
            <label htmlFor="group-status">{tr('状态', 'Status')}</label>
            <select
              id="group-status"
              className="select"
              value={status}
              onChange={(event) => setStatus(event.target.value)}
            >
              <option value="">{tr('全部', 'All')}</option>
              <option value="draft">{tr('草稿', 'Draft')}</option>
              <option value="active">{tr('启用', 'Active')}</option>
              <option value="archived">{tr('已归档', 'Archived')}</option>
            </select>
          </div>
          <button className="btn" type="submit" style={{ marginTop: 18 }}>
            {tr('查询', 'Search')}
          </button>
        </form>
      </div>

      {loading && !data ? (
        <LoadingPanel title={tr('正在读取模型目录…', 'Loading model catalog…')} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={
            appliedQuery || status
              ? tr('没有匹配的模型组', 'No matching model groups')
              : tr('还没有模型组', 'No model groups yet')
          }
          desc={tr(
            '创建模型组（草稿）→ 在详情里添加上游部署 → 激活 → 发布给用户/组。',
            'Create a draft model group, add upstream deployments in its detail page, activate it, then publish to users or groups.',
          )}
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr('新建模型组', 'New model group')}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr('对外模型名', 'Client model')}</th>
                <th>{tr('显示名', 'Display name')}</th>
                <th>Canonical</th>
                <th>{tr('状态', 'Status')}</th>
                <th>{tr('部署', 'Deployments')}</th>
                <th>{tr('发布', 'Published')}</th>
                <th>{tr('工具默认', 'Tool defaults')}</th>
                <th>{tr('告警', 'Warnings')}</th>
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
                      <span className="hint">{tr('未发布', 'Not published')}</span>
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
            {tr(`共 ${data.total} 个模型组`, `${data.total} model groups`)}
          </div>
        </div>
      )}

      {modalOpen && (
        <Modal
          title={tr('新建模型组', 'New model group')}
          sub={tr(
            '先创建草稿；在详情添加部署后激活/发布',
            'Creates a draft first; add deployments in the detail page, then activate and publish',
          )}
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={saving}>
                {tr('取消', 'Cancel')}
              </button>
              <button className="btn primary" type="submit" form="create-group-form" disabled={saving}>
                {saving ? tr('创建中…', 'Creating…') : tr('创建草稿', 'Create draft')}
              </button>
            </>
          }
        >
          <form id="create-group-form" className="grid" onSubmit={(event) => void handleCreate(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-client-name">{tr('对外模型名', 'Client model')}</label>
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
                <label htmlFor="group-display-name">{tr('显示名', 'Display name')}</label>
                <input
                  id="group-display-name"
                  className="input"
                  placeholder={tr('例如 DeepSeek Chat', 'e.g. DeepSeek Chat')}
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
                placeholder={tr(
                  '必须与部署上游模型的 canonical 一致',
                  "Must match the canonical of the deployment's upstream model",
                )}
                value={draft.canonical}
                onChange={(event) => setDraft({ ...draft, canonical: event.target.value })}
              />
              {fieldErrors.canonical && <span className="field-error">{fieldErrors.canonical}</span>}
            </div>
            <div className="field">
              <label htmlFor="group-description">{tr('描述', 'Description')}</label>
              <input
                id="group-description"
                className="input"
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-capabilities">{tr('能力标签（逗号分隔）', 'Capabilities (comma-separated)')}</label>
                <input
                  id="group-capabilities"
                  className="input"
                  value={draft.capabilities}
                  onChange={(event) => setDraft({ ...draft, capabilities: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="group-protocols">{tr('协议（逗号分隔）', 'Protocols (comma-separated)')}</label>
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
                <label htmlFor="group-context">{tr('上下文窗口', 'Context window')}</label>
                <input
                  id="group-context"
                  className="input"
                  type="number"
                  value={draft.context_window}
                  onChange={(event) => setDraft({ ...draft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="group-max-output">{tr('最大输出', 'Max output')}</label>
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
