import '#components/async-action-item.css'

import {
  AlertCircle,
  Check,
  Clock3,
  LoaderCircle,
  Trash2,
  X,
} from 'lucide-react'

import {
  type AsyncAction,
  isAsyncActionFailureStatus,
} from '#async-action-feedback'
import { CloseChecksList } from '#components/close-checks-list'
import type { CloseWorktreeChecks } from '#session-operation-types'

type AsyncActionItemProps = {
  action: AsyncAction
  form: 'toast' | 'row' | 'line'
  time?: string
  onRetry?: () => void
  onDetails?: () => void
  onDismiss?: () => void
  onCloseKeepWorktree?: () => void
  onDiscardChanges?: () => void
  onReviewDiscardAgain?: () => void
  onOpenInTerminal?: () => void
  onShowOutput?: () => void
  onOpenSession?: () => void
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
    ? 'The worktree is clear'
    : `Left in the worktree: ${remaining.join(', ')}`
}

function copy(action: AsyncAction) {
  const pr =
    action.pullRequestNumber === undefined
      ? ''
      : `PR #${String(action.pullRequestNumber)}`
  const labels = (() => {
    switch (action.status) {
      case 'reviewDocumentActionRunning':
        return {
          title: `${action.reviewDocumentAction ?? 'Review action'} in progress`,
          line: `${action.reviewDocumentAction ?? 'Review action'} in progress`,
        }
      case 'reviewDocumentActionSucceeded':
        return {
          title: `${action.reviewDocumentAction ?? 'Review action'} completed`,
          line: `${action.reviewDocumentAction ?? 'Review action'} completed`,
        }
      case 'reviewDocumentActionFailed':
        return {
          title: `${action.reviewDocumentAction ?? 'Review action'} failed`,
          line: `${action.reviewDocumentAction ?? 'Review action'} failed`,
        }
      case 'merging':
        return action.kind === 'merge'
          ? { title: `Merging ${pr}`, line: `Merging ${pr}…` }
          : { title: '', line: '' }
      case 'merged':
        return action.kind === 'merge'
          ? { title: `Merged ${pr}`, line: `Merged ${pr}` }
          : { title: '', line: '' }
      case 'alreadyMerged':
        return action.kind === 'merge'
          ? {
              title: `${pr} was already merged`,
              line: `${pr} was already merged`,
            }
          : { title: '', line: '' }
      case 'queued':
        return action.kind === 'merge'
          ? {
              title: `Added ${pr} to the merge queue`,
              line: `Added ${pr} to the merge queue`,
            }
          : { title: '', line: '' }
      case 'mergeFailed':
        return action.kind === 'merge'
          ? { title: `Merge failed · ${pr}`, line: `Merge of ${pr} failed` }
          : { title: '', line: '' }
      case 'closing':
        return action.kind === 'close'
          ? { title: 'Closing session', line: 'Closing session…' }
          : { title: '', line: '' }
      case 'closed':
        return action.kind === 'close'
          ? { title: 'Closed session', line: 'Closed session' }
          : { title: '', line: '' }
      case 'closeFailed':
        return action.kind === 'close'
          ? { title: 'Close failed', line: 'Close failed' }
          : { title: '', line: '' }
      case 'deleting':
        return action.kind === 'discard'
          ? { title: 'Deleting worktree', line: 'Deleting worktree…' }
          : { title: '', line: '' }
      case 'deleted':
        return action.kind === 'discard'
          ? {
              title: 'Closed session and deleted worktree',
              line: 'Closed session and deleted worktree',
            }
          : { title: '', line: '' }
      case 'deleteChanged':
      case 'deleteBlocked':
        return action.kind === 'discard'
          ? {
              title: 'Did not delete worktree',
              line: 'Did not delete worktree',
            }
          : { title: '', line: '' }
      case 'deleteFailed':
        return action.kind === 'discard'
          ? { title: 'Delete failed', line: 'Delete failed' }
          : { title: '', line: '' }
      case 'rewinding':
        return action.kind === 'rewind'
          ? { title: 'Rewinding session', line: 'Rewinding session…' }
          : { title: '', line: '' }
      case 'rewound':
        return action.kind === 'rewind'
          ? { title: 'Rewound session', line: 'Rewound session' }
          : { title: '', line: '' }
      case 'rewindFailed':
        return action.kind === 'rewind'
          ? { title: 'Rewind failed', line: 'Rewind failed' }
          : { title: '', line: '' }
      case 'runningCommand':
        return { title: 'Running command', line: 'Running command…' }
      case 'ranCommand':
        return { title: 'Ran command', line: 'Ran command' }
      case 'commandFailed':
        return action.exitCode === undefined
          ? { title: 'Command failed', line: 'Command failed' }
          : {
              title: `Command failed · exit ${String(action.exitCode)}`,
              line: `Command failed (exit ${String(action.exitCode)})`,
            }
      case 'commandTimedOut':
        return { title: 'Command timed out', line: 'Command timed out' }
      case 'launching':
        return { title: 'Starting session', line: 'Starting session' }
      case 'launched':
        return { title: 'Session started', line: 'Session started' }
      case 'launchFailed':
        return {
          title: 'Could not start session',
          line: 'Could not start session',
        }
    }
  })()
  const icon =
    action.status === 'merging' ||
    action.status === 'closing' ||
    action.status === 'deleting' ||
    action.status === 'rewinding' ||
    action.status === 'runningCommand' ||
    action.status === 'reviewDocumentActionRunning' ||
    action.status === 'launching'
      ? 'running'
      : action.status === 'merged' ||
          action.status === 'closed' ||
          action.status === 'deleted' ||
          action.status === 'rewound' ||
          action.status === 'ranCommand' ||
          action.status === 'reviewDocumentActionSucceeded' ||
          action.status === 'launched'
        ? 'success'
        : action.status === 'queued'
          ? 'queued'
          : isAsyncActionFailureStatus(action.status)
            ? 'failure'
            : 'neutral'
  return { ...labels, icon }
}

export function AsyncActionItem({
  action,
  form,
  time,
  onRetry,
  onDetails,
  onDismiss,
  onCloseKeepWorktree,
  onDiscardChanges,
  onReviewDiscardAgain,
  onOpenInTerminal,
  onShowOutput,
  onOpenSession,
}: AsyncActionItemProps) {
  const text = copy(action)
  const Icon =
    action.status === 'commandTimedOut'
      ? Clock3
      : text.icon === 'running'
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
  const hasCloseChecks =
    (action.kind === 'close' || action.kind === 'discard') &&
    action.checks !== undefined
  const closeFailedWithChecks =
    action.kind === 'close' && action.status === 'closeFailed' && hasCloseChecks
  const canDiscard =
    closeFailedWithChecks &&
    action.discardable === true &&
    action.checks?.commitInspectionUnavailable === false
  const showDiscardControls = closeFailedWithChecks && canDiscard
  const discardChanged =
    action.kind === 'discard' && action.status === 'deleteChanged'
  const discardBlocked =
    action.kind === 'discard' && action.status === 'deleteBlocked'
  const discardFailed =
    action.kind === 'discard' && action.status === 'deleteFailed'
  const canRetry = action.kind !== 'rewind' && action.retryable !== false
  const canShowCommandOutput =
    action.kind === 'command' &&
    (action.status === 'commandFailed' || action.status === 'commandTimedOut')
  const closeSummary =
    action.kind === 'close' &&
    action.status === 'closeFailed' &&
    action.checks !== undefined
      ? closeFailureSummary(action.checks)
      : undefined
  const launchInitialMessage = action.initialMessage?.split(/\r?\n/u)[0]
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
        size={form === 'line' ? 13 : form === 'row' ? 14 : 15}
      />
      <div className="async-action-item-content">
        <strong>{label}</strong>
        {form !== 'line' && action.kind !== 'command' ? (
          action.kind === 'launch' ? null : (
            <span className="async-action-session">{action.sessionTitle}</span>
          )
        ) : null}
        {action.kind === 'launch' && form !== 'line' ? (
          <>
            {launchInitialMessage !== undefined &&
            launchInitialMessage.length > 0 ? (
              <span className="async-action-launch-message">
                {launchInitialMessage}
              </span>
            ) : null}
            {action.repo !== undefined && action.repo.length > 0 ? (
              <code className="async-action-launch-repo">{action.repo}</code>
            ) : null}
          </>
        ) : null}
        {action.kind === 'command' && form !== 'line' ? (
          <code className="async-action-command">{action.command}</code>
        ) : null}
        {action.status === 'commandTimedOut' && form !== 'line' ? (
          <span className="async-action-command-timeout">
            Stopped after 30 s.
          </span>
        ) : null}
        {action.message !== undefined ? <span>{action.message}</span> : null}
        {closeSummary !== undefined && form !== 'row' ? (
          <span className="async-action-close-summary">{closeSummary}</span>
        ) : action.error !== undefined &&
          (!hasCloseChecks || action.status === 'deleteFailed') ? (
          <span className="async-action-error">{action.error}</span>
        ) : null}
        {form === 'row' && hasCloseChecks ? (
          <CloseChecksList checks={action.checks} />
        ) : null}
        {canShowCommandOutput && form === 'toast' ? (
          <div className="async-action-controls async-action-controls--command-output">
            <button type="button" onClick={onShowOutput}>
              Show output
            </button>
          </div>
        ) : null}
        {showDiscardControls && form !== 'line' ? (
          <>
            <span className="async-action-control-caption">Close anyway</span>
            <div className="async-action-controls async-action-controls--close-options">
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
              <button
                className="async-action-discard"
                type="button"
                aria-haspopup="dialog"
                onClick={onDiscardChanges}
              >
                <Trash2 aria-hidden="true" size={13} />
                Discard changes…
              </button>
            </div>
            <div className="async-action-controls async-action-controls--close-secondary">
              {canRetry ? (
                <button type="button" onClick={onRetry}>
                  Retry
                </button>
              ) : null}
              <button
                type="button"
                onClick={form === 'row' ? onOpenInTerminal : onDetails}
              >
                {form === 'row' ? 'Open in terminal' : 'Details'}
              </button>
            </div>
          </>
        ) : null}
        {showDiscardControls && form === 'line' ? (
          <>
            <div className="async-action-controls async-action-controls--close-options">
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
              <button
                className="async-action-discard"
                type="button"
                aria-haspopup="dialog"
                onClick={onDiscardChanges}
              >
                <Trash2 aria-hidden="true" size={13} />
                Discard changes…
              </button>
            </div>
            <div className="async-action-controls async-action-controls--close-secondary">
              {canRetry ? (
                <button type="button" onClick={onRetry}>
                  Retry
                </button>
              ) : null}
              <button type="button" onClick={onDetails}>
                Details
              </button>
            </div>
          </>
        ) : null}
        {isFailure &&
        !canShowCommandOutput &&
        form === 'toast' &&
        !showDiscardControls &&
        (discardChanged || discardBlocked || discardFailed) ? (
          <div className="async-action-controls async-action-controls--discard">
            {discardChanged ? (
              <>
                <button type="button" onClick={onReviewDiscardAgain}>
                  Review again
                </button>
                <button type="button" onClick={onCloseKeepWorktree}>
                  Close and keep worktree
                </button>
              </>
            ) : discardBlocked ? (
              <>
                <button type="button" onClick={onRetry}>
                  Retry
                </button>
                <button
                  className="async-action-open-terminal"
                  type="button"
                  onClick={onOpenInTerminal}
                >
                  Open in terminal
                </button>
              </>
            ) : (
              <>
                <button type="button" onClick={onRetry}>
                  Retry
                </button>
                <button type="button" onClick={onDetails}>
                  Details
                </button>
              </>
            )}
          </div>
        ) : null}
        {isFailure &&
        !canShowCommandOutput &&
        form === 'toast' &&
        !showDiscardControls &&
        !discardChanged &&
        !discardBlocked &&
        !discardFailed &&
        (canRetry || hasCloseChecks) ? (
          <div className="async-action-controls">
            {hasCloseChecks ? (
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
            ) : null}
            {canRetry ? (
              <button type="button" onClick={onRetry}>
                Retry
              </button>
            ) : null}
            {action.kind === 'merge' ||
            action.kind === 'launch' ||
            hasCloseChecks ? (
              <button type="button" onClick={onDetails}>
                Details
              </button>
            ) : null}
          </div>
        ) : null}
        {canShowCommandOutput && form === 'row' ? (
          <div className="async-action-controls async-action-controls--command-output">
            <button type="button" onClick={onShowOutput}>
              Show output
            </button>
          </div>
        ) : null}
        {action.kind === 'launch' &&
        action.status === 'launched' &&
        form !== 'line' &&
        onOpenSession !== undefined ? (
          <div className="async-action-controls">
            <button type="button" onClick={onOpenSession}>
              Open
            </button>
          </div>
        ) : null}
        {isFailure &&
        !canShowCommandOutput &&
        form === 'row' &&
        !showDiscardControls &&
        (discardChanged || discardBlocked || discardFailed) ? (
          <div className="async-action-controls async-action-controls--discard">
            {discardChanged ? (
              <>
                <button type="button" onClick={onReviewDiscardAgain}>
                  Review again
                </button>
                <button type="button" onClick={onCloseKeepWorktree}>
                  Close and keep worktree
                </button>
              </>
            ) : discardBlocked ? (
              <>
                <button type="button" onClick={onRetry}>
                  Retry
                </button>
                <button
                  className="async-action-open-terminal"
                  type="button"
                  onClick={onOpenInTerminal}
                >
                  Open in terminal
                </button>
                <button type="button" onClick={onCloseKeepWorktree}>
                  Close and keep worktree
                </button>
              </>
            ) : (
              <button type="button" onClick={onRetry}>
                Retry
              </button>
            )}
          </div>
        ) : null}
        {isFailure &&
        !canShowCommandOutput &&
        form === 'row' &&
        !showDiscardControls &&
        !discardChanged &&
        !discardBlocked &&
        !discardFailed &&
        (canRetry || hasCloseChecks) ? (
          <div className="async-action-controls">
            {hasCloseChecks ? (
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
            ) : null}
            {canRetry ? (
              <button type="button" onClick={onRetry}>
                Retry
              </button>
            ) : null}
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
        {isFailure &&
        !canShowCommandOutput &&
        form === 'line' &&
        !showDiscardControls &&
        (discardChanged || discardBlocked || discardFailed) ? (
          <div className="async-action-controls async-action-controls--discard">
            {discardChanged ? (
              <>
                <button type="button" onClick={onReviewDiscardAgain}>
                  Review again
                </button>
                <button type="button" onClick={onCloseKeepWorktree}>
                  Close and keep worktree
                </button>
              </>
            ) : (
              <button type="button" onClick={onRetry}>
                Retry
              </button>
            )}
          </div>
        ) : null}
        {isFailure &&
        !canShowCommandOutput &&
        form === 'line' &&
        !showDiscardControls &&
        !discardChanged &&
        !discardBlocked &&
        !discardFailed &&
        (canRetry || hasCloseChecks) ? (
          <div className="async-action-controls">
            {hasCloseChecks ? (
              <button type="button" onClick={onCloseKeepWorktree}>
                Close and keep worktree
              </button>
            ) : null}
            {canRetry ? (
              <button
                className="async-action-line-retry"
                type="button"
                onClick={onRetry}
              >
                Retry
              </button>
            ) : null}
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
      {(form === 'row' || form === 'line') && time !== undefined ? (
        <time className="async-action-item-time">
          {form === 'line' ? ` · ${time}` : time}
        </time>
      ) : null}
    </div>
  )
}
