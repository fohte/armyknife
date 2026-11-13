import type { Result } from 'neverthrow'
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
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
  id: number
  operation: SessionOperation
  status: 'sending' | 'sent'
}

type ConfirmationTimeout = {
  key: string
  timeout: ReturnType<typeof setTimeout>
}

type SessionOperationContextValue = {
  pendingOperations: readonly PendingSessionOperation[]
  run: (
    operation: SessionOperation,
    options?: RunOptions,
  ) => Promise<Result<unknown, SessionOperationError>>
  confirm: (operation: SessionOperation) => void
}

const STATE_CONFIRMATION_TIMEOUT_MS = 10_000

const SessionOperationContext =
  createContext<SessionOperationContextValue | null>(null)

function useOperationRunner(): SessionOperationContextValue {
  const [pendingOperations, setPendingOperations] = useState<
    PendingSessionOperation[]
  >([])
  const confirmationTimeouts = useRef(new Map<number, ConfirmationTimeout>())
  const nextPendingOperationId = useRef(0)
  const [, startOperationTransition] = useTransition()
  const clearConfirmationTimeout = useCallback((key: string) => {
    confirmationTimeouts.current.forEach(({ key: pendingKey, timeout }, id) => {
      if (pendingKey !== key) return
      clearTimeout(timeout)
      confirmationTimeouts.current.delete(id)
    })
  }, [])
  const confirm = useCallback(
    (operation: SessionOperation) => {
      const key = sessionOperationKey(operation)
      clearConfirmationTimeout(key)
      setPendingOperations((current) =>
        current.filter(
          (pending) => sessionOperationKey(pending.operation) !== key,
        ),
      )
    },
    [clearConfirmationTimeout],
  )
  const expireUnconfirmedOperation = useCallback(
    (operation: SessionOperation, pendingOperationId: number) => {
      const key = sessionOperationKey(operation)
      const timeout = setTimeout(() => {
        confirmationTimeouts.current.delete(pendingOperationId)
        setPendingOperations((current) =>
          current.filter((pending) => pending.id !== pendingOperationId),
        )
      }, STATE_CONFIRMATION_TIMEOUT_MS)
      confirmationTimeouts.current.set(pendingOperationId, { key, timeout })
    },
    [],
  )
  useEffect(
    () => () => {
      confirmationTimeouts.current.forEach(({ timeout }) => {
        clearTimeout(timeout)
      })
      confirmationTimeouts.current.clear()
    },
    [],
  )
  const run = useCallback<SessionOperationContextValue['run']>(
    async (operation, options = {}) => {
      const key = sessionOperationKey(operation)
      const pendingOperationId = nextPendingOperationId.current
      nextPendingOperationId.current += 1
      setPendingOperations((current) => [
        ...current,
        { id: pendingOperationId, operation, status: 'sending' },
      ])
      startOperationTransition(() => {
        options.optimisticDispatchers?.forEach((dispatch) => {
          dispatch(operation)
        })
      })
      const result = await submitSessionOperation(operation)
      if (result.isErr()) {
        clearConfirmationTimeout(key)
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
          pending.id === pendingOperationId
            ? { ...pending, status: 'sent' }
            : pending,
        ),
      )
      if (requiresStateConfirmation(operation)) {
        expireUnconfirmedOperation(operation, pendingOperationId)
      }
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
    [clearConfirmationTimeout, confirm, expireUnconfirmedOperation],
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
