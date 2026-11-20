import '#components/async-action-item.css'

import { AlertCircle, Check, Clock3, LoaderCircle, X } from 'lucide-react'

import type { AsyncAction } from '#async-action-feedback'

type AsyncActionItemProps = {
  action: AsyncAction
  form: 'toast' | 'row' | 'line'
  onRetry?: () => void
  onDetails?: () => void
  onDismiss?: () => void
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
  return (
    <div
      className={`async-action-item async-action-item--${form} async-action-item--${text.icon}`}
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
        {action.error !== undefined ? (
          <span className="async-action-error">{action.error}</span>
        ) : null}
        {isFailure && form === 'toast' ? (
          <div className="async-action-controls">
            <button type="button" onClick={onRetry}>
              Retry
            </button>
            <button type="button" onClick={onDetails}>
              Details
            </button>
          </div>
        ) : null}
        {isFailure && form === 'row' ? (
          <button
            className="async-action-retry"
            type="button"
            onClick={onRetry}
          >
            Retry
          </button>
        ) : null}
        {isFailure && form === 'line' ? (
          <button
            className="async-action-line-retry"
            type="button"
            onClick={onRetry}
          >
            Retry
          </button>
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
