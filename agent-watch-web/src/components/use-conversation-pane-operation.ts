import { type RefObject, useCallback, useState } from 'react'

import { createQueuedPromptId } from '#queued-prompt-id'
import type { useSessionOperations } from '#session-operation-context'
import {
  createSessionOperation,
  type SessionOperation,
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
  operationType: string
  message: string
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
          current.operationType === 'message'
            ? current
            : {
                sessionId,
                operationType: action,
                message: operationResult.error.message,
              },
        )
        return Promise.resolve(false)
      }
      const operation = operationResult.value
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
              current?.sessionId === sessionId &&
              current.operationType === operation.type
                ? null
                : current,
            )
            clearErrors()
            return true
          },
          (requestError) => {
            setSessionOperationError((current) =>
              current?.sessionId === sessionId &&
              current.operationType === 'message' &&
              operation.type !== 'message'
                ? current
                : {
                    sessionId,
                    operationType: operation.type,
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
