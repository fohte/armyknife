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
    | { kind: 'action'; action: string }
    | {
        kind: 'operation'
        key: string
        type: SessionOperation['type']
      }
  message: string
}

function operationErrorIdentity(operation: SessionOperation) {
  switch (operation.type) {
    case 'message':
      return {
        kind: 'operation' as const,
        key: 'message',
        type: operation.type,
      }
    case 'proposal_answer':
      return {
        kind: 'operation' as const,
        key: JSON.stringify([operation.type, operation.seq]),
        type: operation.type,
      }
    case 'todo_resolve':
      return {
        kind: 'operation' as const,
        key: JSON.stringify([operation.type, operation.seq]),
        type: operation.type,
      }
    default:
      return {
        kind: 'operation' as const,
        key: sessionOperationKey(operation),
        type: operation.type,
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
  const [sessionOperationError, setSessionOperationError] =
    useState<SessionOperationError | null>(null)
  const error =
    sessionOperationError?.sessionId === sessionId
      ? sessionOperationError.message
      : null
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
        setSessionOperationError((current) =>
          current?.sessionId === sessionId &&
          current.identity.kind === 'operation' &&
          current.identity.type === 'message'
            ? current
            : {
                sessionId,
                identity: { kind: 'action', action },
                message: operationResult.error.message,
              },
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
            setSessionOperationError((current) =>
              current?.sessionId !== sessionId
                ? current
                : current.identity.kind === 'operation' &&
                    ((current.identity.type === 'message' &&
                      operation.type === 'message') ||
                      current.identity.key === identity.key)
                  ? null
                  : current,
            )
            clearErrors()
            return true
          },
          (requestError) => {
            setSessionOperationError((current) =>
              current?.sessionId === sessionId &&
              current.identity.kind === 'operation' &&
              current.identity.type === 'message' &&
              operation.type !== 'message'
                ? current
                : {
                    sessionId,
                    identity,
                    message: requestError.message,
                  },
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
