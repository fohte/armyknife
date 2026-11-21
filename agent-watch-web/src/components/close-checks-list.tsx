import '#components/close-checks-list.css'

import { AlertCircle, Check } from 'lucide-react'

import type { CloseWorktreeChecks } from '#session-operation-types'

type CheckRow = {
  label: string
  detail: string | null
  state: string
  remaining: boolean
}

export function CloseChecksList({ checks }: { checks: CloseWorktreeChecks }) {
  const rows: CheckRow[] = [
    {
      label: 'Uncommitted changes',
      detail: checks.files.length > 0 ? checks.files.join(' · ') : null,
      state:
        checks.files.length === 0
          ? 'None'
          : `${String(checks.files.length)} ${checks.files.length === 1 ? 'file' : 'files'}`,
      remaining: checks.files.length > 0,
    },
    {
      label: checks.branch === null ? 'Branch' : `Branch ${checks.branch}`,
      detail:
        checks.merged === false && checks.reason !== null
          ? checks.reason
          : null,
      state:
        checks.branch === null
          ? 'None'
          : checks.merged === null
            ? 'Not checked'
            : checks.merged
              ? 'Merged'
              : 'Not merged',
      remaining: checks.branch !== null && checks.merged !== true,
    },
    {
      label: 'Running processes',
      detail: checks.processInspectionUnavailable
        ? 'Could not inspect processes in this worktree.'
        : checks.processes.length > 0
          ? checks.processes
              .map(({ pid, name }) => `${String(pid)} ${name}`)
              .join(' · ')
          : null,
      state: checks.processInspectionUnavailable
        ? 'Unavailable'
        : checks.processes.length === 0
          ? 'None'
          : `${String(checks.processes.length)} running`,
      remaining:
        checks.processInspectionUnavailable || checks.processes.length > 0,
    },
  ]

  return (
    <div className="close-checks-list" role="group" aria-label="Close checks">
      {rows.map(({ label, detail, state, remaining }) => {
        const Icon = remaining ? AlertCircle : Check
        return (
          <div
            className={`close-checks-list__row${remaining ? ' is-remaining' : ' is-clear'}`}
            key={label}
          >
            <Icon
              className="close-checks-list__icon"
              aria-hidden="true"
              size={12}
              strokeWidth={2.5}
            />
            <span className="close-checks-list__content">
              <span className="close-checks-list__label">{label}</span>
              {detail !== null ? (
                <code className="close-checks-list__detail">{detail}</code>
              ) : null}
            </span>
            <span className="close-checks-list__state">{state}</span>
          </div>
        )
      })}
      <code className="close-checks-list__path">{checks.path}</code>
    </div>
  )
}
