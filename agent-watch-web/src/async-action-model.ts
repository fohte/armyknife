import type { CloseWorktreeChecks } from '#session-operation-types'

export type AsyncActionStatus =
  | 'runningCommand'
  | 'ranCommand'
  | 'commandFailed'
  | 'commandTimedOut'
  | 'merging'
  | 'merged'
  | 'alreadyMerged'
  | 'queued'
  | 'mergeFailed'
  | 'closing'
  | 'closed'
  | 'closeFailed'
  | 'rewinding'
  | 'rewound'
  | 'rewindFailed'

export type AsyncAction = {
  id: number
  sessionId: string
  sessionTitle: string
  kind: 'merge' | 'close' | 'command' | 'rewind'
  status: AsyncActionStatus
  command?: string
  commandOutputIndex?: number
  exitCode?: number
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  timelineIndex?: number
  message?: string
  error?: string
  checks?: CloseWorktreeChecks
  createdAt: number
  completedAt?: number
}

export type AsyncActionOutcome =
  | {
      status: 'ranCommand' | 'commandTimedOut'
      commandOutputIndex?: number
    }
  | {
      status: 'commandFailed'
      exitCode?: number
      error?: string
      commandOutputIndex?: number
    }
  | { status: 'merged' | 'alreadyMerged' | 'queued'; message?: string }
  | { status: 'closed' | 'rewound' }
  | {
      status: 'mergeFailed' | 'closeFailed' | 'rewindFailed'
      error: string
      checks?: CloseWorktreeChecks
    }

export type StartAsyncActionOptions = {
  kind: 'merge' | 'close' | 'command' | 'rewind'
  sessionId: string
  sessionTitle: string
  command?: string
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  timelineIndex?: number
  run: () => Promise<AsyncActionOutcome>
  onRetry?: (retry: () => void, actionId: number) => void
  onCloseKeepWorktree?: () => void
  onOpenInTerminal?: () => void
}

export function isAsyncActionFailureStatus(status: AsyncActionStatus) {
  return (
    status === 'mergeFailed' ||
    status === 'closeFailed' ||
    status === 'rewindFailed' ||
    status === 'commandFailed' ||
    status === 'commandTimedOut'
  )
}

export function isAsyncActionRunningStatus(status: AsyncActionStatus) {
  return (
    status === 'merging' ||
    status === 'closing' ||
    status === 'rewinding' ||
    status === 'runningCommand'
  )
}

export function formatAsyncActionTime(timestamp: number) {
  const date = new Date(timestamp)
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`
}

export function runningStatusFor(kind: AsyncAction['kind']): AsyncActionStatus {
  switch (kind) {
    case 'merge':
      return 'merging'
    case 'close':
      return 'closing'
    case 'command':
      return 'runningCommand'
    case 'rewind':
      return 'rewinding'
  }
}
