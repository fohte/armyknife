import type { LaterPreset } from '#session-navigation'
import type { JsonValue, RewindMode } from '#types'

type PermissionAction = 'approve' | 'reject' | 'retry' | 'dismiss'

const REVIEW_DOCUMENT_ACTIONS = [
  'post_comment',
  'create_pr',
  'post_replies',
  'update_issue',
] as const

export type ReviewDocumentAction = (typeof REVIEW_DOCUMENT_ACTIONS)[number]

export type ExpectedPullRequestTarget =
  { kind: 'new' } | { kind: 'existing'; number: number } | { kind: 'unknown' }

export function isReviewDocumentAction(
  value: unknown,
): value is ReviewDocumentAction {
  return (
    typeof value === 'string' &&
    (REVIEW_DOCUMENT_ACTIONS as readonly string[]).includes(value)
  )
}

export type SessionOperation =
  | { type: 'today'; sessionId: string; isToday: boolean }
  | {
      type: 'later'
      sessionId: string
      preset: LaterPreset
      expectedDeadline: string | null
    }
  | { type: 'back_now'; sessionId: string }
  | {
      type: 'close'
      sessionId: string
      keepWorktree: boolean
      stopAndClose?: boolean
      discard?: ConfirmedDiscard
    }
  | { type: 'focus'; sessionId: string }
  | { type: 'resume'; sessionId: string }
  | { type: 'interrupt'; sessionId: string }
  | { type: 'vibe'; sessionId: string; enabled: boolean }
  | { type: 'title'; sessionId: string; title: string }
  | {
      type: 'message'
      sessionId: string
      operationId?: string
      content: string
      attachments?: unknown[]
      queuedPromptId?: string
      messageCountBefore?: number
    }
  | {
      type: 'command_preview' | 'command_execute'
      sessionId: string
      messageText: string
      commands: string[]
    }
  | {
      type: 'answer'
      sessionId: string
      requestId: string
      answers?: Record<string, string>
      skip?: boolean
    }
  | {
      type: 'proposal_answer'
      sessionId: string
      seq: number
      choice: number | null
      comment: string | null
      expectedBody?: JsonValue
      attachments?: unknown[]
    }
  | {
      type: 'todo_resolve'
      sessionId: string
      seq: number
      resolution: 'completed' | 'rejected'
      comment: string | null
      expectedTitle?: string
      expectedDetail?: string | null
      attachments?: unknown[]
    }
  | {
      type: 'todo_fold'
      sessionId: string
      seq: number
      folded: boolean
    }
  | {
      type: 'permission'
      sessionId: string
      action: Exclude<PermissionAction, 'retry'>
      requestId: string
    }
  | { type: 'permission_retry'; sessionId: string; seq: number }
  | { type: 'permission_dismiss'; sessionId: string; seq: number }
  | { type: 'primary_task'; sessionId: string; taskId: string }
  | {
      type: 'merge'
      sessionId: string
      number: number
      owner: string
      repo: string
      expectedHeadSha: string
    }
  | {
      type: 'rewind'
      sessionId: string
      messageId: string
      mode: RewindMode
    }
  | {
      type: 'review_document_save' | 'review_document_finish'
      sessionId: string
      id: string
      revision: string
      frontmatter: JsonValue
      body: string
    }
  | {
      type: 'review_document_action'
      sessionId: string
      id: string
      revision: string
      frontmatter: JsonValue
      body: string
      action: ReviewDocumentAction
      expectedPullRequestTarget?: ExpectedPullRequestTarget
    }

export type CloseChangeStatus = 'M' | 'A' | 'D' | 'R' | '??'

export type ConfirmedDiscard = {
  path: string
  branch: string | null
  changes: { status: CloseChangeStatus; path: string }[]
  commits: string[]
}

export type CloseWorktreeChecks = {
  path: string
  files: string[]
  changes: { status: CloseChangeStatus; path: string }[]
  commits: { sha: string; subject: string }[]
  commitInspectionUnavailable: boolean
  branch: string | null
  merged: boolean | null
  reason: string | null
  processes: { pid: number; name: string }[]
  processInspectionUnavailable: boolean
}

export type SessionOperationError = Error & {
  code?: string | null
  checks?: CloseWorktreeChecks
  retryable?: boolean
  revision?: string
}
function operationKey(operation: SessionOperation) {
  if (operation.type === 'permission') {
    return `${operation.sessionId}:permission:${operation.action}:${operation.requestId}`
  }
  if (operation.type === 'permission_retry') {
    return `${operation.sessionId}:permission_retry:${String(operation.seq)}`
  }
  if (operation.type === 'permission_dismiss') {
    return `${operation.sessionId}:permission_dismiss:${String(operation.seq)}`
  }
  if (operation.type === 'merge') {
    return `${operation.sessionId}:merge:${String(operation.number)}:${operation.owner}/${operation.repo}`
  }
  if (operation.type === 'today') {
    return `${operation.sessionId}:today:${String(operation.isToday)}`
  }
  if (operation.type === 'later') {
    return `${operation.sessionId}:later:${operation.preset}:${operation.expectedDeadline ?? 'none'}`
  }
  if (operation.type === 'primary_task') {
    return `${operation.sessionId}:primary_task:${operation.taskId}`
  }
  if (operation.type === 'review_document_action') {
    return `${operation.sessionId}:review_document_action:${operation.id}`
  }
  if (operation.type === 'title') {
    return `${operation.sessionId}:title:${operation.title.trim()}`
  }
  if (operation.type === 'answer') {
    return `${operation.sessionId}:answer:${operation.requestId}`
  }
  if (operation.type === 'message') {
    return `${operation.sessionId}:message:${operation.operationId ?? String(operation.messageCountBefore ?? 'unknown')}`
  }
  if (
    operation.type === 'command_preview' ||
    operation.type === 'command_execute'
  ) {
    return `${operation.sessionId}:${operation.type}:${JSON.stringify([operation.messageText, operation.commands])}`
  }
  if (operation.type === 'proposal_answer') {
    return `${operation.sessionId}:proposal_answer:${String(operation.seq)}:${String(operation.choice)}:${operation.comment ?? ''}${operation.expectedBody === undefined ? '' : `:${JSON.stringify(operation.expectedBody)}`}`
  }
  if (operation.type === 'todo_resolve') {
    return `${operation.sessionId}:todo_resolve:${String(operation.seq)}:${operation.resolution}:${operation.comment ?? ''}${operation.expectedTitle === undefined ? '' : `:${JSON.stringify([operation.expectedTitle, operation.expectedDetail])}`}`
  }
  if (operation.type === 'todo_fold') {
    return `${operation.sessionId}:todo_fold:${String(operation.seq)}:${String(operation.folded)}`
  }
  return `${operation.sessionId}:${operation.type}`
}

export function sessionOperationKey(operation: SessionOperation) {
  return operationKey(operation)
}
