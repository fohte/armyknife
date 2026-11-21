import '#components/async-action-item.css'

import { AlertCircle, Check, Clock3, LoaderCircle, X } from 'lucide-react'

import type { AsyncAction } from '#async-action-feedback'
import type { CloseWorktreeChecks } from '#session-operation-types'

type AsyncActionItemProps = {
  action: AsyncAction
  form: 'toast' | 'row' | 'line'
  onRetry?: () => void
  onDetails?: () => void
  onDismiss?: () => void
  onCloseKeepWorktree?: () => void
  onOpenInTerminal?: () => void
}

function closeFailureSummary(checks: CloseWorktreeChecks) {
  const remaining: string[] = []
  if (checks.files.length > 0) {
    remaining.push(
      `${String(checks.files.length)} uncommitted ${checks.files.length === 1 ? 'file' : 'files'}`,
    )
  }
  if (checks.merged === false) remaining.push('branch not merged')
  else if (checks.branch !== null && checks.merged === null) {
    remaining.push('branch merge status unknown')
  }
  if (checks.processes.length > 0) {
    remaining.push(
      `${String(checks.processes.length)} running ${checks.processes.length === 1 ? 'process' : 'processes'}`,
    )
  } else if (checks.processInspectionUnavailable) {
    remaining.push('process inspection unavailable')
  }
  return remaining.length === 0
    ? 'The worktree is clear.'
    : `Left in the worktree: ${remaining.join(', ')}.`
}

function CloseChecksList({ checks }: { checks: CloseWorktreeChecks }) {
  const branchState =
    checks.merged === null
      ? 'Not checked'
      : checks.merged
        ? 'Merged'
        : 'Not merged'
  const processState = checks.processInspectionUnavailable
    ? 'Unavailable'
    : checks.processes.length === 0
      ? 'None'
      : `${String(checks.processes.length)} running`
  return (
    <div
      className="async-action-close-checks"
      role="group"
      aria-label="Close checks"
    >
      <div>
        <span>Uncommitted changes</span>
        <strong>
          {checks.files.length === 0
            ? 'None'
            : `${String(checks.files.length)} ${checks.files.length === 1 ? 'file' : 'files'}`}
        </strong>
        {checks.files.length > 0 ? (
          <code>{checks.files.join(' · ')}</code>
        ) : null}
      </div>
      <div>
        <span>
          {checks.branch === null ? 'Branch' : `Branch ${checks.branch}`}
        </span>
        <strong>{branchState}</strong>
        {checks.merged === false && checks.reason !== null ? (
          <code>{checks.reason}</code>
        ) : null}
      </div>
      <div>
        <span>Running processes</span>
        <strong>{processState}</strong>
        {checks.processes.length > 0 ? (
          <code>
            {checks.processes
              .map(({ pid, name }) => `${String(pid)} ${name}`)
              .join(' · ')}
          </code>
        ) : checks.processInspectionUnavailable ? (
          <code>Could not inspect processes in this worktree.</code>
        ) : null}
      </div>
      <code className="async-action-worktree-path">{checks.path}</code>
    </div>
  )
}

function copy(action: AsyncAction) {
  const pr =
    action.pullRequestNumber === undefined
      ? ''
      : `PR #${String(action.pullRequestNumber)}`
  const title =
    action.kind === 'merge'
      ? {
          merging: `Merging ${pr}`,
          merged: `Merged ${pr}`,
          alreadyMerged: `${pr} was already merged`,
          queued: `Added ${pr} to the merge queue`,
          mergeFailed: `Merge failed · ${pr}`,
          closing: '',
          closed: '',
          closeFailed: '',
        }[action.status]
      : {
          merging: '',
          merged: '',
          alreadyMerged: '',
          queued: '',
          mergeFailed: '',
          closing: 'Closing session',
          closed: 'Closed session',
          closeFailed: 'Close failed',
        }[action.status]
  const line =
    action.kind === 'merge'
      ? {
          merging: `Merging ${pr}…`,
          merged: `Merged ${pr}`,
          alreadyMerged: `${pr} was already merged`,
          queued: `Added ${pr} to the merge queue`,
          mergeFailed: `Merge of ${pr} failed`,
          closing: '',
          closed: '',
          closeFailed: '',
        }[action.status]
      : {
          merging: '',
          merged: '',
          alreadyMerged: '',
          queued: '',
          mergeFailed: '',
          closing: 'Closing session…',
          closed: 'Closed session',
          closeFailed: 'Close failed',
        }[action.status]
  const icon =
    action.status === 'merging' || action.status === 'closing'
      ? 'running'
      : action.status === 'merged' || action.status === 'closed'
        ? 'success'
        : action.status === 'queued'
          ? 'queued'
          : action.status === 'mergeFailed' || action.status === 'closeFailed'
            ? 'failure'
            : 'neutral'
  return { title, line, icon }
}

export function AsyncActionItem({
  action,
  form,
  onRetry,
  onDetails,
  onDismiss,
  onCloseKeepWorktree,
  onOpenInTerminal,
}: AsyncActionItemProps) {
  const text = copy(action)
  const Icon =
    text.icon === 'running'
      ? LoaderCircle
      : text.icon === 'success'
        ? Check
        : text.icon === 'queued'
          ? Clock3
          : text.icon === 'failure'
            ? AlertCircle
            : Check
  const isFailure = text.icon === 'failure'
  const isRunning = text.icon === 'running'
  const isQueue = text.icon === 'queued'
  const label = form === 'line' ? text.line : text.title
  const hasCloseChecks = action.kind === 'close' && action.checks !== undefined
  const closeSummary =
    action.kind === 'close' && action.checks !== undefined
      ? closeFailureSummary(action.checks)
      : undefined
  return (
    <div
      className={`async-action-item async-action-item--${form} async-action-item--${text.icon}${hasCloseChecks ? ' has-close-worktree' : ''}`}
      role={
        isFailure
          ? 'alert'
          : form === 'line' || form === 'toast'
            ? 'status'
            : undefined
      }
    >
      <Icon
        className={isRunning ? 'async-action-spinner' : undefined}
        aria-hidden="true"
        size={form === 'line' ? 13 : 15}
      />
      <div className="async-action-item-content">
        <strong>{label}</strong>
        {form !== 'line' ? (
          <span className="async-action-session">{action.sessionTitle}</span>
        ) : null}
        {action.message !== undefined ? <span>{action.message}</span> : null}
        {closeSummary !== undefined && form !== 'row' ? (
          <span className="async-action-close-summary">{closeSummary}</span>
        ) : action.error !== undefined && !hasCloseChecks ? (
          <span className="async-action-error">{action.error}</span>
        ) : null}
        {form === 'row' && action.checks !== undefined ? (
          <CloseChecksList checks={action.checks} />
        ) : null}
        {isFailure && form === 'toast' ? (
          <div className="async-action-controls">
            {hasCloseChecks ? (
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
            ) : null}
            <button type="button" onClick={onRetry}>
              Retry
            </button>
            {action.kind !== 'close' || hasCloseChecks ? (
              <button type="button" onClick={onDetails}>
                Details
              </button>
            ) : null}
          </div>
        ) : null}
        {isFailure && form === 'row' ? (
          <div className="async-action-controls">
            {hasCloseChecks ? (
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
            ) : null}
            <button type="button" onClick={onRetry}>
              Retry
            </button>
            {hasCloseChecks ? (
              <button
                className="async-action-open-terminal"
                type="button"
                onClick={onOpenInTerminal}
              >
                Open in terminal
              </button>
            ) : null}
          </div>
        ) : null}
        {isFailure && form === 'line' ? (
          <div className="async-action-controls">
            {hasCloseChecks ? (
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
            ) : null}
            <button
              className="async-action-line-retry"
              type="button"
              onClick={onRetry}
            >
              Retry
            </button>
          </div>
        ) : null}
      </div>
      {form === 'toast' ? (
        <button
          className="async-action-dismiss"
          type="button"
          aria-label="Dismiss"
          onClick={onDismiss}
        >
          <X aria-hidden="true" size={14} />
        </button>
      ) : null}
      {isQueue ? (
        <span className="async-action-timer" aria-hidden="true" />
      ) : null}
    </div>
  )
}
