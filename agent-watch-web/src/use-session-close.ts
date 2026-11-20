import type { Result } from 'neverthrow'
import { useCallback, useState } from 'react'

import { useAsyncActionStarter } from '#async-action-feedback'
import { useSessionConversationCache } from '#components/session-conversation-cache'
import { clearSessionAttachments } from '#components/use-composer-message'
import {
  type SessionOperation,
  type SessionOperationError,
  shouldOfferKeepWorktreeRetry,
} from '#session-operations'
import type { Session } from '#types'

type UseSessionCloseOptions = {
  sessions: readonly Session[]
  runSessionOperation: (
    operation: SessionOperation,
  ) => Promise<Result<unknown, SessionOperationError>>
  closeDoneSession: (sessionId: string, keepWorktree?: boolean) => void
  setCloseRetrySessionId: (sessionId: string | null) => void
  setError: (error: string | null) => void
}

export function useSessionClose({
  sessions,
  runSessionOperation,
  closeDoneSession,
  setCloseRetrySessionId,
  setError,
}: UseSessionCloseOptions) {
  const [confirmationSessionId, setConfirmationSessionId] = useState<
    string | null
  >(null)
  const sessionConversationCache = useSessionConversationCache()
  const startAsyncAction = useAsyncActionStarter((feedbackError) => {
    setError(feedbackError.message)
  })
  const confirmationSession = sessions.find(
    (session) => session.id === confirmationSessionId,
  )

  const closeNonDoneSession = useCallback(
    (sessionId: string, keepWorktree: boolean) => {
      const session = sessions.find((candidate) => candidate.id === sessionId)
      if (session === undefined) return
      startAsyncAction({
        kind: 'close',
        sessionId,
        sessionTitle: session.title,
        run: async () =>
          runSessionOperation({
            type: 'close',
            sessionId,
            keepWorktree,
            stopAndClose: true,
          }).then((result) =>
            result.match(
              () => {
                sessionConversationCache?.clearCachedConversation(sessionId)
                clearSessionAttachments(sessionId)
                setError(null)
                setCloseRetrySessionId(null)
                return { status: 'closed' as const }
              },
              (operationError) => {
                setError(operationError.message)
                setCloseRetrySessionId(
                  shouldOfferKeepWorktreeRetry(operationError)
                    ? sessionId
                    : null,
                )
                return {
                  status: 'closeFailed' as const,
                  error: operationError.message,
                }
              },
            ),
          ),
      })
    },
    [
      startAsyncAction,
      runSessionOperation,
      sessionConversationCache,
      sessions,
      setCloseRetrySessionId,
      setError,
    ],
  )

  const requestCloseSession = useCallback(
    (sessionId: string) => {
      const session = sessions.find((candidate) => candidate.id === sessionId)
      if (session === undefined) return
      if (session.category === 'done') {
        closeDoneSession(sessionId)
      } else {
        setConfirmationSessionId(sessionId)
      }
    },
    [closeDoneSession, sessions],
  )

  const cancelCloseConfirmation = useCallback(() => {
    setConfirmationSessionId(null)
  }, [])

  const confirmCloseSession = useCallback(
    (sessionId: string) => {
      setConfirmationSessionId(null)
      closeNonDoneSession(sessionId, false)
    },
    [closeNonDoneSession],
  )

  const closeSessionKeepWorktree = useCallback(
    (sessionId: string) => {
      const session = sessions.find((candidate) => candidate.id === sessionId)
      if (session === undefined) return
      if (session.category === 'done') {
        closeDoneSession(sessionId, true)
      } else {
        closeNonDoneSession(sessionId, true)
      }
    },
    [closeDoneSession, closeNonDoneSession, sessions],
  )

  return {
    confirmationSession,
    requestCloseSession,
    cancelCloseConfirmation,
    confirmCloseSession,
    closeSessionKeepWorktree,
  }
}
