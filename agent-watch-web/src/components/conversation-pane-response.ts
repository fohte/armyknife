import { err, ok, Result, ResultAsync } from 'neverthrow'

import type {
  BackgroundTask,
  ChatItem,
  ConversationResponse,
  SessionActions,
} from '#types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isUserContentPart(value: unknown): boolean {
  if (!isRecord(value) || typeof value['type'] !== 'string') return false
  if (value['type'] === 'text') return typeof value['text'] === 'string'
  if (value['type'] === 'file') return typeof value['name'] === 'string'
  if (value['type'] !== 'image') return false
  return (
    typeof value['source'] === 'string' &&
    (value['source'].startsWith('/api/images/') ||
      /^data:image\/(png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(
        value['source'],
      )) &&
    typeof value['available'] === 'boolean'
  )
}

function isProposalOption(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value['label'] === 'string' &&
    (value['description'] === null ||
      typeof value['description'] === 'string') &&
    Array.isArray(value['pros']) &&
    value['pros'].every((pro) => typeof pro === 'string') &&
    Array.isArray(value['cons']) &&
    value['cons'].every((con) => typeof con === 'string') &&
    typeof value['recommended'] === 'boolean'
  )
}

function isProposalAnswer(value: unknown): boolean {
  return (
    isRecord(value) &&
    (value['choice'] === null ||
      (typeof value['choice'] === 'number' &&
        Number.isInteger(value['choice']) &&
        value['choice'] >= 0)) &&
    (value['comment'] === null || typeof value['comment'] === 'string')
  )
}

function isChatItem(value: unknown): value is ChatItem {
  if (!isRecord(value) || typeof value['type'] !== 'string') return false
  switch (value['type']) {
    case 'user':
    case 'assistant':
      return typeof value['text'] === 'string'
    case 'queued_user':
    case 'user_with_content':
      return (
        Array.isArray(value['content']) &&
        value['content'].every(isUserContentPart) &&
        (value['from'] === undefined || typeof value['from'] === 'string')
      )
    case 'peer':
      return (
        (value['direction'] === 'from' || value['direction'] === 'to') &&
        typeof value['session_id'] === 'string' &&
        (value['session_title'] === undefined ||
          typeof value['session_title'] === 'string') &&
        Array.isArray(value['content']) &&
        value['content'].every(isUserContentPart)
      )
    case 'subagent_report':
      return (
        typeof value['agent_id'] === 'string' &&
        typeof value['agent_name'] === 'string' &&
        (value['description'] === undefined ||
          typeof value['description'] === 'string') &&
        typeof value['text'] === 'string'
      )
    case 'event':
      return (
        (value['kind'] === 'armyknife' ||
          value['kind'] === 'system' ||
          value['kind'] === 'hook') &&
        typeof value['text'] === 'string' &&
        typeof value['failed'] === 'boolean'
      )
    case 'tool_summary':
      return (
        typeof value['summary'] === 'string' &&
        (value['operations'] === undefined ||
          (Array.isArray(value['operations']) &&
            value['operations'].every(
              (operation) =>
                isRecord(operation) &&
                typeof operation['tool'] === 'string' &&
                (operation['target'] === undefined ||
                  typeof operation['target'] === 'string'),
            )))
      )
    case 'tool_in_progress':
      return (
        typeof value['tool'] === 'string' &&
        (value['target'] === undefined || typeof value['target'] === 'string')
      )
    case 'question':
      return (
        typeof value['question'] === 'string' &&
        (value['header'] === undefined ||
          typeof value['header'] === 'string') &&
        Array.isArray(value['choices']) &&
        value['choices'].every(
          (choice) =>
            isRecord(choice) &&
            typeof choice['label'] === 'string' &&
            (choice['description'] === undefined ||
              typeof choice['description'] === 'string'),
        ) &&
        (value['multi_select'] === undefined ||
          typeof value['multi_select'] === 'boolean') &&
        (value['answer'] === null || typeof value['answer'] === 'string')
      )
    case 'proposal':
      return (
        typeof value['seq'] === 'number' &&
        Number.isInteger(value['seq']) &&
        value['seq'] > 0 &&
        typeof value['title'] === 'string' &&
        (value['summary'] === null || typeof value['summary'] === 'string') &&
        Array.isArray(value['options']) &&
        value['options'].every(isProposalOption) &&
        (value['answer'] === null || isProposalAnswer(value['answer'])) &&
        typeof value['resolved'] === 'boolean'
      )
    case 'permission':
      return (
        typeof value['tool'] === 'string' &&
        typeof value['description'] === 'string'
      )
    case 'decision':
      return (
        typeof value['seq'] === 'number' &&
        typeof value['title'] === 'string' &&
        typeof value['why'] === 'string' &&
        Array.isArray(value['considered']) &&
        value['considered'].every((item) => typeof item === 'string') &&
        typeof value['easyToChange'] === 'string' &&
        (value['review'] === null ||
          value['review'] === 'ok' ||
          value['review'] === 'change') &&
        typeof value['createdAt'] === 'string'
      )
    default:
      return false
  }
}

function isBackgroundTask(value: unknown): value is BackgroundTask {
  return (
    isRecord(value) &&
    typeof value['id'] === 'string' &&
    typeof value['description'] === 'string' &&
    typeof value['elapsedSeconds'] === 'number' &&
    (value['kind'] === 'subagent' || value['kind'] === 'background_task') &&
    (value['agentType'] === undefined || typeof value['agentType'] === 'string')
  )
}

export function isConversationResponse(
  value: unknown,
): value is ConversationResponse {
  return (
    isRecord(value) &&
    Array.isArray(value['messages']) &&
    value['messages'].every(isChatItem) &&
    Array.isArray(value['backgroundTasks']) &&
    value['backgroundTasks'].every(isBackgroundTask)
  )
}

export function isSessionActions(value: unknown): value is SessionActions {
  return (
    isRecord(value) &&
    typeof value['message'] === 'boolean' &&
    typeof value['answerQuestion'] === 'boolean' &&
    typeof value['interrupt'] === 'boolean' &&
    typeof value['openInTerminal'] === 'boolean' &&
    (value['permission'] === undefined ||
      (isRecord(value['permission']) &&
        typeof value['permission']['approve'] === 'boolean' &&
        typeof value['permission']['reject'] === 'boolean'))
  )
}

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Request failed', { cause: error })
}

function errorFromResponse(response: Response) {
  const fallback =
    response.statusText || `Request failed (${String(response.status)})`
  return ResultAsync.fromPromise(response.text(), toError).map((body) => {
    const parsed =
      body === ''
        ? null
        : Result.fromThrowable(() => JSON.parse(body) as unknown, toError)()
    const message =
      parsed !== null &&
      parsed.isOk() &&
      isRecord(parsed.value) &&
      typeof parsed.value['error'] === 'string'
        ? parsed.value['error']
        : fallback
    return new Error(message)
  })
}

export function requestJson<T>(
  url: string,
  validate: (value: unknown) => value is T,
) {
  return ResultAsync.fromPromise(fetch(url), toError).andThen((response) => {
    if (!response.ok) {
      return errorFromResponse(response).andThen((error) => err(error))
    }
    return ResultAsync.fromPromise(response.json(), toError).andThen((body) =>
      validate(body)
        ? ok(body)
        : err(new Error('The server returned an invalid response')),
    )
  })
}

export function requestAction(
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
  ).andThen((response) => {
    if (response.ok) return ok(undefined)
    return errorFromResponse(response).andThen((error) => err(error))
  })
}
