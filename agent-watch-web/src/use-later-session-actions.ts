import type { Result } from 'neverthrow'
import type { Dispatch, SetStateAction } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'

import { getLaterPresetTimes } from '#session-actions'
import type { HistoryMode, SessionHistoryState } from '#session-history'
import type { LaterPreset, LaterPresetTimes } from '#session-navigation'
import type {
  SessionOperation,
  SessionOperationError,
} from '#session-operations'
import type { Session } from '#types'

type Navigate = (
  update: (current: SessionHistoryState) => SessionHistoryState,
  mode?: HistoryMode,
) => void

type UseLaterSessionActionsOptions = {
  isMobile: boolean
  sessions: readonly Session[]
  navigableSessions: readonly Session[]
  pendingOperations: readonly { operation: SessionOperation }[]
  navigate: Navigate
  runSessionOperation: (
    operation: SessionOperation,
  ) => Promise<Result<unknown, SessionOperationError>>
  setError: Dispatch<SetStateAction<string | null>>
  setLaterExpanded: Dispatch<SetStateAction<boolean>>
}

export function useLaterSessionActions({
  isMobile,
  sessions,
  navigableSessions,
  pendingOperations,
  navigate,
  runSessionOperation,
  setError,
  setLaterExpanded,
}: UseLaterSessionActionsOptions) {
  const [laterSessionId, setLaterSessionId] = useState<string | null>(null)
  const [laterTimes, setLaterTimes] = useState<LaterPresetTimes | null>(null)
  const [laterTimesError, setLaterTimesError] = useState(false)
  const laterSubmissionInFlight = useRef(false)
  const laterRequestId = useRef(0)

  const openLater = useCallback(
    (id: string) => {
      const requestId = ++laterRequestId.current
      setLaterTimes(null)
      setLaterTimesError(false)
      setLaterSessionId(id)
      setError(null)
      void getLaterPresetTimes().match(
        (times) => {
          if (requestId === laterRequestId.current) {
            setLaterTimes(times)
          }
        },
        () => {
          if (requestId === laterRequestId.current) {
            setLaterTimesError(true)
            setError('Could not load Later return times.')
          }
        },
      )
    },
    [setError],
  )

  const closeLater = useCallback(() => {
    laterRequestId.current += 1
    setLaterSessionId(null)
  }, [])

  const beginLaterMutation = useCallback(() => {
    if (laterSubmissionInFlight.current) return false
    laterSubmissionInFlight.current = true
    return true
  }, [])

  const finishLaterMutation = useCallback(() => {
    laterSubmissionInFlight.current = false
  }, [])

  const submitLater = useCallback(
    (sessionId: string, preset: LaterPreset) => {
      const expectedDeadline = laterTimes?.[preset].deadline
      if (expectedDeadline === undefined || !beginLaterMutation()) return

      void runSessionOperation({
        type: 'later',
        sessionId,
        preset,
        expectedDeadline,
      }).then((result) => {
        result.match(
          () => {
            finishLaterMutation()
            closeLater()
            if (isMobile) {
              setLaterExpanded(false)
              navigate(
                (current) => ({
                  ...current,
                  selectedId:
                    navigableSessions.find(
                      (session) => session.id !== sessionId,
                    )?.id ?? null,
                  conversationOpen: false,
                }),
                'push',
              )
            }
            setError(null)
          },
          () => {
            finishLaterMutation()
            openLater(sessionId)
            setError('Could not move the session to Later.')
          },
        )
      })
    },
    [
      beginLaterMutation,
      closeLater,
      finishLaterMutation,
      isMobile,
      laterTimes,
      navigableSessions,
      navigate,
      openLater,
      runSessionOperation,
      setError,
      setLaterExpanded,
    ],
  )

  const submitBackNow = useCallback(
    (sessionId: string) => {
      if (!beginLaterMutation()) return
      void runSessionOperation({ type: 'back_now', sessionId }).then(
        (result) => {
          result.match(
            () => {
              finishLaterMutation()
              closeLater()
              setError(null)
            },
            () => {
              finishLaterMutation()
              setError('Could not return the session now.')
            },
          )
        },
      )
    },
    [
      beginLaterMutation,
      closeLater,
      finishLaterMutation,
      runSessionOperation,
      setError,
    ],
  )

  useEffect(() => {
    if (
      laterSessionId !== null &&
      !sessions.some((session) => session.id === laterSessionId)
    ) {
      closeLater()
    }
  }, [closeLater, laterSessionId, sessions])

  const laterMutationPending = pendingOperations.some(
    ({ operation }) =>
      operation.sessionId === laterSessionId &&
      (operation.type === 'later' || operation.type === 'back_now'),
  )

  return {
    laterSessionId,
    laterTimes,
    laterTimesError,
    laterSubmissionInFlight,
    laterMutationPending,
    openLater,
    closeLater,
    submitLater,
    submitBackNow,
  }
}
