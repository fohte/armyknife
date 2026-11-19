import { useCallback, useMemo, useOptimistic, useRef } from 'react'

import { useSessionOperations } from '#session-operation-context'
import {
  applySessionOperation,
  isSessionListOperation,
  isSessionOperationConfirmed,
  type SessionOperation,
} from '#session-operations'
import type { Session } from '#types'

export function useAppSessionOperations(sessions: readonly Session[]) {
  const sessionSnapshot = useRef<readonly Session[]>([])
  const { pendingOperations, run, confirm } = useSessionOperations()
  const pendingOperationsRef = useRef(pendingOperations)
  pendingOperationsRef.current = pendingOperations
  const sessionBaseWithPendingOperations = useMemo(
    () =>
      pendingOperations
        .filter(({ operation }) => isSessionListOperation(operation))
        .reduce(
          (current, { operation }) => applySessionOperation(current, operation),
          sessions,
        ),
    [pendingOperations, sessions],
  )
  const [optimisticSessions, dispatchSessionOperation] = useOptimistic(
    sessionBaseWithPendingOperations,
    applySessionOperation,
  )
  const runSessionOperation = useCallback(
    (operation: SessionOperation) =>
      run(operation, {
        optimisticDispatchers: [dispatchSessionOperation],
        ...(isSessionListOperation(operation)
          ? {
              refresh: () => {
                if (
                  isSessionOperationConfirmed(
                    operation,
                    sessionSnapshot.current,
                  )
                ) {
                  confirm(operation)
                }
                return Promise.resolve()
              },
            }
          : {}),
      }),
    [confirm, dispatchSessionOperation, run],
  )
  const recordSessionSnapshot = useCallback(
    (snapshot: readonly Session[]) => {
      sessionSnapshot.current = snapshot
      for (const pending of pendingOperationsRef.current) {
        if (
          pending.status === 'sent' &&
          isSessionListOperation(pending.operation) &&
          isSessionOperationConfirmed(pending.operation, snapshot)
        ) {
          confirm(pending.operation)
        }
      }
    },
    [confirm],
  )

  return {
    pendingOperations,
    optimisticSessions,
    dispatchSessionOperation,
    runSessionOperation,
    recordSessionSnapshot,
  }
}
