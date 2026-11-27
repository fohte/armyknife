import type { CloseWorktreeChecks } from '#session-operation-types'

export type AsyncActionStatus =
  | 'reviewDocumentActionRunning'
  | 'reviewDocumentActionSucceeded'
  | 'reviewDocumentActionFailed'
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
  | 'launching'
  | 'launched'
  | 'launchFailed'

export type AsyncAction = {
  id: number
  sessionId: string
  sessionTitle: string
  kind: 'merge' | 'close' | 'command' | 'rewind' | 'review_document' | 'launch'
  status: AsyncActionStatus
  command?: string
  initialMessage?: string
  repo?: string
  reviewDocumentAction?: string
  commandOutputIndex?: number
  exitCode?: number
  pullRequestNumber?: number
  pullRequestOwner?: string
  pullRequestRepo?: string
  pullRequestHeadSha?: string
  timelineIndex?: number
  message?: string
  error?: string
  retryable?: boolean
  checks?: CloseWorktreeChecks
  createdAt: number
  completedAt?: number
}

export type AsyncActionOutcome =
  | { status: 'reviewDocumentActionSucceeded' }
  | {
      status: 'reviewDocumentActionFailed'
      error: string
      retryable?: boolean
    }
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
  | { status: 'launched' }
  | { status: 'launchFailed'; error: string }
  | {
      status: 'mergeFailed' | 'closeFailed' | 'rewindFailed'
      error: string
      checks?: CloseWorktreeChecks
    }

type StartAsyncActionBaseOptions = {
  sessionId: string
  sessionTitle: string
  command?: string
  initialMessage?: string
  repo?: string
  reviewDocumentAction?: AsyncAction['reviewDocumentAction']
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

export type StartAsyncActionOptions =
  | (StartAsyncActionBaseOptions & {
      kind: 'launch'
      onOpenSession: () => void
    })
  | (StartAsyncActionBaseOptions & {
      kind: Exclude<AsyncAction['kind'], 'launch'>
    })

export function isAsyncActionFailureStatus(status: AsyncActionStatus) {
  return (
    status === 'mergeFailed' ||
    status === 'closeFailed' ||
    status === 'rewindFailed' ||
    status === 'commandFailed' ||
    status === 'commandTimedOut' ||
    status === 'reviewDocumentActionFailed' ||
    status === 'launchFailed'
  )
}

export function isAsyncActionRunningStatus(status: AsyncActionStatus) {
  return (
    status === 'merging' ||
    status === 'closing' ||
    status === 'rewinding' ||
    status === 'runningCommand' ||
    status === 'reviewDocumentActionRunning' ||
    status === 'launching'
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
    case 'review_document':
      return 'reviewDocumentActionRunning'
    case 'launch':
      return 'launching'
  }
}
