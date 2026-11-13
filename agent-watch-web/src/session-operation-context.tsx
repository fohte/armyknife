import type { Result } from 'neverthrow'
import {
  createContext,
  useCallback,
  useContext,
  useState,
  useTransition,
} from 'react'

import {
  requiresStateConfirmation,
  type SessionOperation,
  type SessionOperationError,
  sessionOperationKey,
  submitSessionOperation,
} from '#session-operations'

type RunOptions = {
  optimisticDispatchers?: readonly ((operation: SessionOperation) => void)[]
  refresh?: (result: unknown) => Promise<void>
}

export type PendingSessionOperation = {
  operation: SessionOperation
  status: 'sending' | 'sent'
}

type SessionOperationContextValue = {
  pendingOperations: readonly PendingSessionOperation[]
  run: (
    operation: SessionOperation,
    options?: RunOptions,
  ) => Promise<Result<unknown, SessionOperationError>>
  confirm: (operation: SessionOperation) => void
}

const SessionOperationContext =
  createContext<SessionOperationContextValue | null>(null)

function useOperationRunner(): SessionOperationContextValue {
  const [pendingOperations, setPendingOperations] = useState<
    PendingSessionOperation[]
  >([])
  const [, startOperationTransition] = useTransition()
  const confirm = useCallback((operation: SessionOperation) => {
    const key = sessionOperationKey(operation)
    setPendingOperations((current) =>
      current.filter(
        (pending) => sessionOperationKey(pending.operation) !== key,
      ),
    )
  }, [])
  const run = useCallback<SessionOperationContextValue['run']>(
    async (operation, options = {}) => {
      const key = sessionOperationKey(operation)
      setPendingOperations((current) => [
        ...current,
        { operation, status: 'sending' },
      ])
      startOperationTransition(() => {
        options.optimisticDispatchers?.forEach((dispatch) => {
          dispatch(operation)
        })
      })
      const result = await submitSessionOperation(operation)
      if (result.isErr()) {
        const failedMessageCountBefore =
          operation.type === 'message'
            ? operation.messageCountBefore
            : undefined
        setPendingOperations((current) =>
          current
            .filter((pending) => sessionOperationKey(pending.operation) !== key)
            .map((pending) => {
              const pendingOperation = pending.operation
              if (
                failedMessageCountBefore === undefined ||
                pendingOperation.type !== 'message' ||
                pendingOperation.sessionId !== operation.sessionId ||
                pendingOperation.messageCountBefore === undefined ||
                pendingOperation.messageCountBefore <= failedMessageCountBefore
              ) {
                return pending
              }
              return {
                ...pending,
                operation: {
                  ...pendingOperation,
                  messageCountBefore: pendingOperation.messageCountBefore - 1,
                },
              }
            }),
        )
        return result
      }
      setPendingOperations((current) =>
        current.map((pending) =>
          sessionOperationKey(pending.operation) === key
            ? { ...pending, status: 'sent' }
            : pending,
        ),
      )
      if (options.refresh !== undefined) {
        const clearPending = () => {
          if (!requiresStateConfirmation(operation)) confirm(operation)
        }
        void options.refresh(result.value).then(clearPending, clearPending)
      } else if (!requiresStateConfirmation(operation)) {
        confirm(operation)
      }
      return result
    },
    [confirm],
  )
  return { pendingOperations, run, confirm }
}

export function SessionOperationProvider({
  children,
}: {
  children: React.ReactNode
}) {
  const { pendingOperations, run, confirm } = useOperationRunner()
  return (
    <SessionOperationContext.Provider
      value={{ pendingOperations, run, confirm }}
    >
      {children}
    </SessionOperationContext.Provider>
  )
}

export function useSessionOperations() {
  const context = useContext(SessionOperationContext)
  const local = useOperationRunner()
  return context ?? local
}
