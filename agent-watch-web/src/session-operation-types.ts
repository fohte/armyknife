import type { LaterPreset } from '#session-navigation'

type PermissionAction = 'approve' | 'reject' | 'retry'

export type SessionOperation =
  | { type: 'today'; sessionId: string; isToday: boolean }
  | {
      type: 'later'
      sessionId: string
      preset: LaterPreset
      expectedDeadline: string | null
    }
  | { type: 'back_now'; sessionId: string }
  | { type: 'close'; sessionId: string; keepWorktree: boolean }
  | { type: 'focus'; sessionId: string }
  | { type: 'interrupt'; sessionId: string }
  | {
      type: 'message'
      sessionId: string
      operationId?: string
      content: string
      attachments?: unknown[]
      messageCountBefore?: number
    }
  | {
      type: 'answer'
      sessionId: string
      requestId: string
      answers?: Record<string, string>
      skip?: boolean
    }
  | { type: 'decision_ok'; sessionId: string; seq: number }
  | {
      type: 'decision_change'
      sessionId: string
      seq: number
      comment: string
    }
  | {
      type: 'proposal_answer'
      sessionId: string
      seq: number
      choice: number | null
      comment: string | null
    }
  | {
      type: 'todo_complete'
      sessionId: string
      seq: number
      comment: string
    }
  | {
      type: 'permission'
      sessionId: string
      action: Exclude<PermissionAction, 'retry'>
      requestId: string
    }
  | { type: 'permission_retry'; sessionId: string; seq: number }
  | { type: 'primary_task'; sessionId: string; taskId: string }
  | {
      type: 'merge'
      sessionId: string
      number: number
      owner: string
      repo: string
      expectedHeadSha: string
    }

export type SessionOperationError = Error & { code?: string | null }
function operationKey(operation: SessionOperation) {
  if (operation.type === 'permission') {
    return `${operation.sessionId}:permission:${operation.action}:${operation.requestId}`
  }
  if (operation.type === 'permission_retry') {
    return `${operation.sessionId}:permission_retry:${String(operation.seq)}`
  }
  if (
    operation.type === 'decision_ok' ||
    operation.type === 'decision_change'
  ) {
    return `${operation.sessionId}:${operation.type}:${String(operation.seq)}`
  }
  if (operation.type === 'merge') {
    return `${operation.sessionId}:merge:${String(operation.number)}`
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
  if (operation.type === 'answer') {
    return `${operation.sessionId}:answer:${operation.requestId}`
  }
  if (operation.type === 'message') {
    return `${operation.sessionId}:message:${operation.operationId ?? String(operation.messageCountBefore ?? 'unknown')}`
  }
  if (operation.type === 'proposal_answer') {
    return `${operation.sessionId}:proposal_answer:${String(operation.seq)}:${String(operation.choice)}:${operation.comment ?? ''}`
  }
  if (operation.type === 'todo_complete') {
    return `${operation.sessionId}:todo_complete:${String(operation.seq)}:${operation.comment}`
  }
  return `${operation.sessionId}:${operation.type}`
}

export function sessionOperationKey(operation: SessionOperation) {
  return operationKey(operation)
}
