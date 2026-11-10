import { err, ok, Result, ResultAsync } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

import { ConversationPaneView } from '#components/conversation-pane-view'
import type { PendingPermissionAction } from '#components/permission-card'
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
        Array.isArray(value['choices']) &&
        value['choices'].every((choice) => typeof choice === 'string') &&
        (value['answer'] === null || typeof value['answer'] === 'string')
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

function requestJson<T>(url: string, validate: (value: unknown) => value is T) {
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
    return errorFromResponse(response).andThen((error) => err(error))
  })
}

type ConversationPaneProps = {
  session: Session
  refreshKey: number
  onSelectSession: (id: string) => void
  isMobile?: boolean
  onBack?: () => void
  onLater?: () => void
}

export function ConversationPane({
  session,
  refreshKey,
  onSelectSession,
  isMobile = false,
  onBack = () => undefined,
  onLater = () => undefined,
}: ConversationPaneProps) {
  const [messages, setMessages] = useState<ChatItem[]>([])
  const [actions, setActions] = useState<SessionActions | null>(null)
  const [actionsReady, setActionsReady] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [pendingPermissionAction, setPendingPermissionAction] =
    useState<PendingPermissionAction>(null)
  const requestGeneration = useRef(0)

  const refresh = useCallback(() => {
    const generation = requestGeneration.current + 1
    requestGeneration.current = generation
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
        if (requestGeneration.current !== generation) return
        if (messagesResult.isOk()) setMessages(messagesResult.value)
        if (actionsResult.isOk()) setActions(actionsResult.value)
        setPendingPermissionAction((pending) => {
          if (pending === null) return null
          const permissionDisappeared =
            messagesResult.isOk() &&
            !messagesResult.value.some(
              (message) => message.type === 'permission',
            )
          const permissionUnavailable =
            actionsResult.isOk() &&
            (actionsResult.value.permission == null ||
              (!actionsResult.value.permission.approve &&
                !actionsResult.value.permission.reject))
          return permissionDisappeared || permissionUnavailable ? null : pending
        })
        const firstError = messagesResult.isErr()
          ? messagesResult.error
          : actionsResult.isErr()
            ? actionsResult.error
            : null
        if (messagesResult.isOk() && actionsResult.isOk()) {
          setActionsReady(true)
        }
        setError(firstError?.message ?? null)
        setLoading(false)
      },
    )
  }, [session.id])

  useEffect(() => {
    refresh()
    return () => {
      requestGeneration.current += 1
    }
  }, [refresh, refreshKey])

  useEffect(() => {
    setPendingPermissionAction(null)
  }, [session.id])

  const onAction = (action: string, payload?: Record<string, unknown>) => {
    const permissionAction =
      action === 'permission/approve'
        ? 'approve'
        : action === 'permission/reject'
          ? 'reject'
          : null
    if (permissionAction !== null) {
      setPendingPermissionAction({
        action: permissionAction,
        status: 'sending',
      })
    }
    return requestAction(session.id, action, payload).match(
      () => {
        if (permissionAction !== null) {
          setPendingPermissionAction({
            action: permissionAction,
            status: 'sent',
          })
        }
        setError(null)
        refresh()
        return true
      },
      (requestError) => {
        if (permissionAction !== null) setPendingPermissionAction(null)
        setError(requestError.message)
        return false
      },
    )
  }

  return (
    <ConversationPaneView
      session={session}
      messages={messages}
      actions={actions}
      actionsReady={actionsReady}
      pendingPermissionAction={pendingPermissionAction}
      loading={loading}
      error={error}
      onClearError={() => {
        setError(null)
      }}
      onAction={onAction}
      onSelectSession={onSelectSession}
      isMobile={isMobile}
      onBack={onBack}
      onLater={onLater}
    />
  )
}
