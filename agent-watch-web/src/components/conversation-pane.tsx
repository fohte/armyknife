import { err, ok, ResultAsync } from 'neverthrow'
import { useCallback, useEffect, useState } from 'react'

import { ConversationPaneView } from '#components/conversation-pane-view'
import type { ChatItem, Session, SessionActions } from '#types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isChatItem(value: unknown): value is ChatItem {
  if (!isRecord(value) || typeof value['type'] !== 'string') return false
  switch (value['type']) {
    case 'user':
    case 'assistant':
      return typeof value['text'] === 'string'
    case 'tool_summary':
      return typeof value['summary'] === 'string'
    case 'question':
      return (
        typeof value['question'] === 'string' &&
        Array.isArray(value['choices']) &&
        value['choices'].every((choice) => typeof choice === 'string')
      )
    case 'permission':
      return (
        typeof value['tool'] === 'string' &&
        typeof value['description'] === 'string'
      )
    default:
      return false
  }
}

function isChatItems(value: unknown): value is ChatItem[] {
  return Array.isArray(value) && value.every(isChatItem)
}

function isSessionActions(value: unknown): value is SessionActions {
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

function requestJson<T>(url: string, validate: (value: unknown) => value is T) {
  return ResultAsync.fromPromise(fetch(url), toError).andThen((response) => {
    if (!response.ok) {
      return ResultAsync.fromPromise(response.json(), toError).andThen((body) =>
        err(
          new Error(
            isRecord(body) && typeof body['error'] === 'string'
              ? body['error']
              : response.statusText,
          ),
        ),
      )
    }
    return ResultAsync.fromPromise(response.json(), toError).andThen((body) =>
      validate(body)
        ? ok(body)
        : err(new Error('The server returned an invalid response')),
    )
  })
}

function requestAction(
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
    return ResultAsync.fromPromise(response.json(), toError).andThen((body) =>
      err(
        new Error(
          isRecord(body) && typeof body['error'] === 'string'
            ? body['error']
            : response.statusText,
        ),
      ),
    )
  })
}

type ConversationPaneProps = {
  session: Session
  refreshKey: number
  onSelectSession: (id: string) => void
}

export function ConversationPane({
  session,
  refreshKey,
  onSelectSession,
}: ConversationPaneProps) {
  const [messages, setMessages] = useState<ChatItem[]>([])
  const [actions, setActions] = useState<SessionActions | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const refresh = useCallback(() => {
    const sessionPath = `/api/sessions/${encodeURIComponent(session.id)}`
    const messagesRequest = requestJson(
      `${sessionPath}/messages`,
      isChatItems,
    ).match(
      (value) => ok(value),
      (requestError) => err(requestError),
    )
    const actionsRequest = requestJson(
      `${sessionPath}/actions`,
      isSessionActions,
    ).match(
      (value) => ok(value),
      (requestError) => err(requestError),
    )
    void Promise.all([messagesRequest, actionsRequest]).then(
      ([messagesResult, actionsResult]) => {
        if (messagesResult.isOk()) setMessages(messagesResult.value)
        if (actionsResult.isOk()) setActions(actionsResult.value)
        const firstError = messagesResult.isErr()
          ? messagesResult.error
          : actionsResult.isErr()
            ? actionsResult.error
            : null
        setError(firstError?.message ?? null)
        setLoading(false)
      },
    )
  }, [session.id])

  useEffect(() => {
    setMessages([])
    setActions(null)
    setLoading(true)
    setError(null)
    refresh()
  }, [refresh, refreshKey])

  const onAction = (action: string, payload?: Record<string, unknown>) => {
    void requestAction(session.id, action, payload).match(
      () => {
        setError(null)
        refresh()
      },
      (requestError) => {
        setError(requestError.message)
      },
    )
  }

  return (
    <ConversationPaneView
      session={session}
      messages={messages}
      actions={actions}
      loading={loading}
      error={error}
      onAction={onAction}
      onSelectSession={onSelectSession}
    />
  )
}
