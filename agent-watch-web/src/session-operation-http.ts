import { err, ok, ResultAsync } from 'neverthrow'

import { errorFromResponse, jsonFromResponse } from '#api-requests'
import { LATER_PRESET_OPTIONS } from '#session-navigation'
import type {
  SessionOperation,
  SessionOperationError,
} from '#session-operation-types'

let nextMessageOperationId = 0

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isStringRecord(value: unknown): value is Record<string, string> {
  return (
    isRecord(value) &&
    Object.values(value).every((entry) => typeof entry === 'string')
  )
}

function toError(error: unknown): SessionOperationError {
  return error instanceof Error
    ? error
    : new Error('Session operation failed', { cause: error })
}

function postAction(
  sessionId: string,
  path: string,
  payload?: Record<string, unknown>,
) {
  return ResultAsync.fromPromise(
    fetch(`/api/sessions/${encodeURIComponent(sessionId)}/${path}`, {
      method: 'POST',
      ...(payload === undefined
        ? {}
        : {
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          }),
    }),
    toError,
  )
}

export function requestAction(
  sessionId: string,
  path: string,
  payload?: Record<string, unknown>,
) {
  return postAction(sessionId, path, payload).andThen((response) => {
    if (response.ok) return ok<undefined, SessionOperationError>(undefined)
    return errorFromResponse(response).andThen((error) =>
      err<undefined, SessionOperationError>(error),
    )
  })
}

export function requestActionJson<T>(
  sessionId: string,
  path: string,
  payload: Record<string, unknown>,
  validate: (value: unknown) => value is T,
) {
  return postAction(sessionId, path, payload).andThen((response) =>
    jsonFromResponse(response, validate),
  )
}

function requestFor(operation: SessionOperation) {
  switch (operation.type) {
    case 'today':
      return { path: 'today', body: { isToday: operation.isToday } }
    case 'later':
      return {
        path: 'later',
        body: {
          preset: operation.preset,
          expectedDeadline: operation.expectedDeadline,
        },
      }
    case 'back_now':
      return { path: 'later', body: { preset: 'back_now' } }
    case 'close':
      return { path: 'close', body: { keepWorktree: operation.keepWorktree } }
    case 'focus':
      return { path: 'focus', body: undefined }
    case 'interrupt':
      return { path: 'interrupt', body: undefined }
    case 'title':
      return { path: 'title', body: { title: operation.title } }
    case 'message':
      return {
        path: 'messages',
        body: {
          content: operation.content,
          ...(operation.attachments === undefined
            ? {}
            : { attachments: operation.attachments }),
        },
      }
    case 'answer':
      return {
        path: 'answer',
        body: {
          requestId: operation.requestId,
          ...(operation.skip === true
            ? { skip: true }
            : { answers: operation.answers }),
        },
      }
    case 'decision_ok':
      return { path: `decisions/${String(operation.seq)}/ok`, body: undefined }
    case 'decision_change':
      return {
        path: `decisions/${String(operation.seq)}/change`,
        body: { comment: operation.comment },
      }
    case 'proposal_answer':
      return {
        path: `proposals/${String(operation.seq)}/answer`,
        body: { choice: operation.choice, comment: operation.comment },
      }
    case 'todo_resolve':
      return {
        path: `todos/${String(operation.seq)}/resolve`,
        body: {
          resolution: operation.resolution,
          comment: operation.comment,
        },
      }
    case 'permission':
      return {
        path: `permission/${operation.action}`,
        body: { requestId: operation.requestId },
      }
    case 'permission_retry':
      return {
        path: `permission-denials/${String(operation.seq)}/retry`,
        body: undefined,
      }
    case 'primary_task':
      return { path: 'primary-task', body: { taskId: operation.taskId } }
    case 'merge':
      return {
        path: `pull-requests/${String(operation.number)}/merge`,
        body: {
          owner: operation.owner,
          repo: operation.repo,
          expectedHeadSha: operation.expectedHeadSha,
        },
      }
  }
}

export function createSessionOperation(
  sessionId: string,
  action: string,
  payload: Record<string, unknown> = {},
) {
  const sequenceFromPath = (prefix: string, suffix: string) => {
    const match = new RegExp(`^${prefix}/(\\d+)/${suffix}$`).exec(action)
    return match === null ? null : Number(match[1])
  }
  const requiredString = (key: string) =>
    typeof payload[key] === 'string' ? payload[key] : null

  switch (action) {
    case 'today':
      return typeof payload['isToday'] === 'boolean'
        ? ok<SessionOperation, Error>({
            type: 'today',
            sessionId,
            isToday: payload['isToday'],
          })
        : errOperation('Invalid Starred operation')
    case 'later':
      if (payload['preset'] === 'back_now') {
        return ok<SessionOperation, Error>({ type: 'back_now', sessionId })
      }
      {
        const preset = LATER_PRESET_OPTIONS.find(
          ({ preset: candidate }) => candidate === payload['preset'],
        )?.preset
        const expectedDeadline = payload['expectedDeadline']
        return preset !== undefined &&
          (expectedDeadline === null || typeof expectedDeadline === 'string')
          ? ok<SessionOperation, Error>({
              type: 'later',
              sessionId,
              preset,
              expectedDeadline,
            })
          : errOperation('Invalid Later operation')
      }
    case 'close':
      return typeof payload['keepWorktree'] === 'boolean'
        ? ok<SessionOperation, Error>({
            type: 'close',
            sessionId,
            keepWorktree: payload['keepWorktree'],
          })
        : errOperation('Invalid close operation')
    case 'focus':
      return ok<SessionOperation, Error>({ type: 'focus', sessionId })
    case 'interrupt':
      return ok<SessionOperation, Error>({ type: 'interrupt', sessionId })
    case 'title': {
      const title = requiredString('title')
      return title === null
        ? errOperation('Invalid title operation')
        : ok<SessionOperation, Error>({ type: 'title', sessionId, title })
    }
    case 'messages':
      return typeof payload['content'] === 'string'
        ? ok<SessionOperation, Error>({
            type: 'message',
            sessionId,
            operationId:
              requiredString('_operationId') ??
              `message-${String(++nextMessageOperationId)}`,
            content: payload['content'],
            ...(Array.isArray(payload['attachments'])
              ? { attachments: payload['attachments'] }
              : {}),
            ...(typeof payload['_messageCountBefore'] === 'number'
              ? { messageCountBefore: payload['_messageCountBefore'] }
              : {}),
          })
        : errOperation('Invalid message operation')
    case 'answer': {
      const requestId = payload['requestId']
      const skip = payload['skip'] === true
      const answers = payload['answers']
      if (typeof requestId !== 'string') {
        return errOperation('Invalid answer operation')
      }
      if (skip) {
        return ok<SessionOperation, Error>({
          type: 'answer',
          sessionId,
          requestId,
          skip: true,
        })
      }
      if (!isStringRecord(answers)) {
        return errOperation('Invalid answer operation')
      }
      return ok<SessionOperation, Error>({
        type: 'answer',
        sessionId,
        requestId,
        answers,
      })
    }
    case 'permission/approve':
    case 'permission/reject':
      return requiredString('requestId') === null
        ? errOperation('Invalid permission operation')
        : ok<SessionOperation, Error>({
            type: 'permission',
            sessionId,
            action: action.endsWith('approve') ? 'approve' : 'reject',
            requestId: requiredString('requestId') ?? '',
          })
    case 'primary-task':
      return requiredString('taskId') === null
        ? errOperation('Invalid primary task operation')
        : ok<SessionOperation, Error>({
            type: 'primary_task',
            sessionId,
            taskId: requiredString('taskId') ?? '',
          })
    default: {
      const decision = sequenceFromPath('decisions', 'ok')
      if (decision !== null) {
        return ok<SessionOperation, Error>({
          type: 'decision_ok',
          sessionId,
          seq: decision,
        })
      }
      const decisionChange = sequenceFromPath('decisions', 'change')
      if (decisionChange !== null && requiredString('comment') !== null) {
        return ok<SessionOperation, Error>({
          type: 'decision_change',
          sessionId,
          seq: decisionChange,
          comment: requiredString('comment') ?? '',
        })
      }
      const proposal = sequenceFromPath('proposals', 'answer')
      if (
        proposal !== null &&
        (typeof payload['choice'] === 'number' || payload['choice'] === null) &&
        (typeof payload['comment'] === 'string' || payload['comment'] === null)
      ) {
        return ok<SessionOperation, Error>({
          type: 'proposal_answer',
          sessionId,
          seq: proposal,
          choice: payload['choice'],
          comment: payload['comment'],
        })
      }
      const todo = sequenceFromPath('todos', 'resolve')
      if (
        todo !== null &&
        (payload['resolution'] === 'completed' ||
          payload['resolution'] === 'rejected') &&
        (payload['comment'] === null ||
          typeof payload['comment'] === 'string' ||
          payload['comment'] === undefined)
      ) {
        return ok<SessionOperation, Error>({
          type: 'todo_resolve',
          sessionId,
          seq: todo,
          resolution: payload['resolution'],
          comment:
            typeof payload['comment'] === 'string' ? payload['comment'] : null,
        })
      }
      const retry = sequenceFromPath('permission-denials', 'retry')
      if (retry !== null) {
        return ok<SessionOperation, Error>({
          type: 'permission_retry',
          sessionId,
          seq: retry,
        })
      }
      return errOperation(`Unsupported session operation: ${action}`)
    }
  }
}

function errOperation(message: string) {
  return err<SessionOperation, Error>(new Error(message))
}

export function submitSessionOperation(operation: SessionOperation) {
  const request = requestFor(operation)
  return postAction(operation.sessionId, request.path, request.body).andThen(
    (response) => {
      if (!response.ok)
        return errorFromResponse(response).andThen((error) => err(error))
      if (operation.type !== 'merge')
        return ok<unknown, SessionOperationError>(undefined)
      return ResultAsync.fromPromise(response.json(), toError)
    },
  )
}
