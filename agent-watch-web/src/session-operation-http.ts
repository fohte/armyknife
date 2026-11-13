import { err, ok, ResultAsync } from 'neverthrow'

import { errorFromResponse } from '#api-requests'
import { isJsonValue } from '#json-value'
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

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((entry) => typeof entry === 'string')
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
    case 'command_preview':
    case 'command_execute':
      return {
        path:
          operation.type === 'command_preview'
            ? 'commands/preview'
            : 'commands/execute',
        body: {
          messageText: operation.messageText,
          commands: operation.commands,
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
    case 'rewind':
      return {
        path: 'rewind',
        body: { messageId: operation.messageId, mode: operation.mode },
      }
    case 'review_document_save':
    case 'review_document_finish':
      return {
        path: `review-document/${operation.type === 'review_document_save' ? 'save' : 'finish'}`,
        body: {
          id: operation.id,
          frontmatter: operation.frontmatter,
          body: operation.body,
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
    case 'review-document/save':
    case 'review-document/finish': {
      const id = requiredString('id')
      const body = requiredString('body')
      const frontmatter = payload['frontmatter']
      if (id === null || body === null || !isJsonValue(frontmatter)) {
        return errOperation('Invalid review document operation')
      }
      return ok<SessionOperation, Error>({
        type:
          action === 'review-document/save'
            ? 'review_document_save'
            : 'review_document_finish',
        sessionId,
        id,
        frontmatter,
        body,
      })
    }
    case 'commands/preview':
    case 'commands/execute': {
      const messageText = requiredString('messageText')
      const commands = payload['commands']
      if (
        messageText === null ||
        !isStringArray(commands) ||
        commands.length === 0
      ) {
        return errOperation('Invalid command operation')
      }
      return ok<SessionOperation, Error>({
        type:
          action === 'commands/preview' ? 'command_preview' : 'command_execute',
        sessionId,
        messageText,
        commands,
      })
    }
    case 'rewind': {
      const messageId = requiredString('messageId')
      const mode = payload['mode']
      if (
        messageId === null ||
        (mode !== 'conversation_and_code' &&
          mode !== 'conversation' &&
          mode !== 'code')
      ) {
        return errOperation('Invalid rewind operation')
      }
      return ok<SessionOperation, Error>({
        type: 'rewind',
        sessionId,
        messageId,
        mode,
      })
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
      if (
        operation.type !== 'merge' &&
        operation.type !== 'rewind' &&
        operation.type !== 'command_preview' &&
        operation.type !== 'command_execute'
      )
        return ok<unknown, SessionOperationError>(undefined)
      return ResultAsync.fromPromise<unknown, SessionOperationError>(
        response.json(),
        toError,
      )
    },
  )
}
