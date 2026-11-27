import { type RefObject, useCallback, useState } from 'react'

import { createQueuedPromptId } from '#queued-prompt-id'
import type { useSessionOperations } from '#session-operation-context'
import {
  createSessionOperation,
  type SessionOperation,
  sessionOperationKey,
} from '#session-operations'
import type { ConversationResponse } from '#types'

type UseConversationPaneOperationOptions = {
  sessionId: string
  conversationRef: RefObject<ConversationResponse>
  nextMessageCountBefore: RefObject<number | null>
  dispatchConversationOperation: (operation: SessionOperation) => void
  onOptimisticSessionOperation?:
    ((operation: SessionOperation) => void) | undefined
  refreshForOperation: (operation: SessionOperation) => Promise<void>
  clearErrors: () => void
  run: ReturnType<typeof useSessionOperations>['run']
}

type SessionOperationError = {
  sessionId: string
  identity:
    { kind: 'action'; action: string } | { kind: 'operation'; key: string }
  message: string
}

function sameErrorIdentity(
  left: SessionOperationError['identity'],
  right: SessionOperationError['identity'],
) {
  if (left.kind === 'action' && right.kind === 'action') {
    return left.action === right.action
  }
  if (left.kind === 'operation' && right.kind === 'operation') {
    return left.key === right.key
  }
  return false
}

function setOperationError(
  errors: SessionOperationError[],
  nextError: SessionOperationError,
) {
  return [
    ...errors.filter(
      (current) =>
        current.sessionId !== nextError.sessionId ||
        !sameErrorIdentity(current.identity, nextError.identity),
    ),
    nextError,
  ]
}

function clearOperationError(
  errors: SessionOperationError[],
  sessionId: string,
  action: string,
  identity: SessionOperationError['identity'],
) {
  return errors.filter((current) => {
    if (current.sessionId !== sessionId) return true
    return current.identity.kind === 'action'
      ? current.identity.action !== action
      : !sameErrorIdentity(current.identity, identity)
  })
}

function latestOperationError(
  errors: SessionOperationError[],
  sessionId: string,
) {
  for (let index = errors.length - 1; index >= 0; index -= 1) {
    const current = errors[index]
    if (current?.sessionId === sessionId) return current.message
  }
  return null
}

function operationErrorIdentity(operation: SessionOperation) {
  switch (operation.type) {
    case 'message':
      return {
        kind: 'operation' as const,
        key: 'message',
      }
    case 'proposal_answer':
      return {
        kind: 'operation' as const,
        key: JSON.stringify([operation.type, operation.seq]),
      }
    case 'todo_resolve':
      return {
        kind: 'operation' as const,
        key: JSON.stringify([operation.type, operation.seq]),
      }
    default:
      return {
        kind: 'operation' as const,
        key: sessionOperationKey(operation),
      }
  }
}

export function useConversationPaneOperation({
  sessionId,
  conversationRef,
  nextMessageCountBefore,
  dispatchConversationOperation,
  onOptimisticSessionOperation,
  refreshForOperation,
  clearErrors,
  run,
}: UseConversationPaneOperationOptions) {
  const [sessionOperationErrors, setSessionOperationErrors] = useState<
    SessionOperationError[]
  >([])
  const error = latestOperationError(sessionOperationErrors, sessionId)
  const onAction = useCallback(
    (action: string, payload?: Record<string, unknown>) => {
      const messageCountBefore =
        action === 'messages'
          ? Math.max(
              conversationRef.current.total,
              nextMessageCountBefore.current ?? 0,
            )
          : undefined
      const operationResult = createSessionOperation(
        sessionId,
        action,
        action === 'messages'
          ? {
              ...payload,
              _queuedPromptId: createQueuedPromptId(),
              _messageCountBefore: messageCountBefore,
            }
          : payload,
      )
      if (operationResult.isErr()) {
        setSessionOperationErrors((current) =>
          setOperationError(current, {
            sessionId,
            identity: { kind: 'action', action },
            message: operationResult.error.message,
          }),
        )
        return Promise.resolve(false)
      }
      const operation = operationResult.value
      const identity = operationErrorIdentity(operation)
      if (operation.type === 'message' && messageCountBefore !== undefined) {
        nextMessageCountBefore.current = messageCountBefore + 1
      }
      const dispatchers = [dispatchConversationOperation]
      if (onOptimisticSessionOperation !== undefined) {
        dispatchers.push(onOptimisticSessionOperation)
      }
      return run(operation, {
        optimisticDispatchers: dispatchers,
        refresh: (_result, confirmedOperation) =>
          refreshForOperation(confirmedOperation),
      }).then((result) =>
        result.match(
          () => {
            setSessionOperationErrors((current) =>
              clearOperationError(current, sessionId, action, identity),
            )
            clearErrors()
            return true
          },
          (requestError) => {
            setSessionOperationErrors((current) =>
              setOperationError(current, {
                sessionId,
                identity,
                message: requestError.message,
              }),
            )
            return false
          },
        ),
      )
    },
    [
      clearErrors,
      conversationRef,
      dispatchConversationOperation,
      nextMessageCountBefore,
      onOptimisticSessionOperation,
      refreshForOperation,
      run,
      sessionId,
    ],
  )

  return { error, onAction }
}
