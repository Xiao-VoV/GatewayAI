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
  if (status === 'active') return <Badge tone="ok">{tr("active_2")}</Badge>
  if (status === 'draft') return <Badge tone="warn">{tr("draft")}</Badge>
  return <Badge>{tr("archived")}</Badge>
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
        setFormError(tr("failed_to_create_the_model_group_please_try_again"))
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
      {loading ? tr("refreshing") : tr("refresh")}
    </button>
  )

  return (
    <>
      <PageHead
        title={tr("model_catalog")}
        desc={tr("client_facing_model_groups_and_deployments_add_deploymen")}
        actions={
          <>
            {refresh}
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr("new_model_group")}
            </button>
          </>
        }
      />

      <div className="card">
        <form className="toolbar" onSubmit={submitSearch}>
          <div className="field" style={{ width: 260 }}>
            <label htmlFor="group-query">{tr("search_2")}</label>
            <input
              id="group-query"
              className="input"
              placeholder={tr("client_model_display_name_canonical")}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
          </div>
          <div className="field" style={{ width: 150 }}>
            <label htmlFor="group-status">{tr("status")}</label>
            <select
              id="group-status"
              className="select"
              value={status}
              onChange={(event) => setStatus(event.target.value)}
            >
              <option value="">{tr("all")}</option>
              <option value="draft">{tr("draft")}</option>
              <option value="active">{tr("active_2")}</option>
              <option value="archived">{tr("archived")}</option>
            </select>
          </div>
          <button className="btn" type="submit" style={{ marginTop: 18 }}>
            {tr("search")}
          </button>
        </form>
      </div>

      {loading && !data ? (
        <LoadingPanel title={tr("loading_model_catalog")} />
      ) : error ? (
        <ErrorPanel error={error} onRetry={reload} />
      ) : !data || data.items.length === 0 ? (
        <EmptyPanel
          title={
            appliedQuery || status
              ? tr("no_matching_model_groups")
              : tr("no_model_groups_yet")
          }
          desc={tr("create_a_draft_model_group_add_upstream_deployments_in_i")}
          action={
            <button className="btn primary" type="button" onClick={openCreate}>
              {tr("new_model_group")}
            </button>
          }
        />
      ) : (
        <div className="table-wrap">
          <table className="table dense">
            <thead>
              <tr>
                <th>{tr("client_model")}</th>
                <th>{tr("display_name")}</th>
                <th>Canonical</th>
                <th>{tr("status")}</th>
                <th>{tr("deployments")}</th>
                <th>{tr("published")}</th>
                <th>{tr("tool_defaults")}</th>
                <th>{tr("warnings")}</th>
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
                      <span className="hint">{tr("not_published")}</span>
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
            {tr("model_groups", { total: data.total })}
          </div>
        </div>
      )}

      {modalOpen && (
        <Modal
          title={tr("new_model_group")}
          sub={tr("creates_a_draft_first_add_deployments_in_the_detail_page")}
          onClose={() => setModalOpen(false)}
          wide
          footer={
            <>
              <button className="btn" type="button" onClick={() => setModalOpen(false)} disabled={saving}>
                {tr("cancel")}
              </button>
              <button className="btn primary" type="submit" form="create-group-form" disabled={saving}>
                {saving ? tr("creating") : tr("create_draft")}
              </button>
            </>
          }
        >
          <form id="create-group-form" className="grid" onSubmit={(event) => void handleCreate(event)} noValidate>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-client-name">{tr("client_model")}</label>
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
                <label htmlFor="group-display-name">{tr("display_name")}</label>
                <input
                  id="group-display-name"
                  className="input"
                  placeholder={tr("e_g_deepseek_chat")}
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
                placeholder={tr("must_match_the_canonical_of_the_deployment_s_upstream_mo")}
                value={draft.canonical}
                onChange={(event) => setDraft({ ...draft, canonical: event.target.value })}
              />
              {fieldErrors.canonical && <span className="field-error">{fieldErrors.canonical}</span>}
            </div>
            <div className="field">
              <label htmlFor="group-description">{tr("description")}</label>
              <input
                id="group-description"
                className="input"
                value={draft.description}
                onChange={(event) => setDraft({ ...draft, description: event.target.value })}
              />
            </div>
            <div className="grid cols-2">
              <div className="field">
                <label htmlFor="group-capabilities">{tr("capabilities_comma_separated_2")}</label>
                <input
                  id="group-capabilities"
                  className="input"
                  value={draft.capabilities}
                  onChange={(event) => setDraft({ ...draft, capabilities: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="group-protocols">{tr("protocols_comma_separated")}</label>
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
                <label htmlFor="group-context">{tr("context_window")}</label>
                <input
                  id="group-context"
                  className="input"
                  type="number"
                  value={draft.context_window}
                  onChange={(event) => setDraft({ ...draft, context_window: event.target.value })}
                />
              </div>
              <div className="field">
                <label htmlFor="group-max-output">{tr("max_output")}</label>
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
