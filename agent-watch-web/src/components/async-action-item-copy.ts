import {
  type AsyncAction,
  isAsyncActionFailureStatus,
} from '#async-action-model'

export function copyAsyncAction(action: AsyncAction) {
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
        if (action.kind === 'discard') {
          return {
            title: 'Closed session and deleted worktree',
            line: 'Closed session and deleted worktree',
          }
        }
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
