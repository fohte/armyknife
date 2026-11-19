import type { Result } from 'neverthrow'
import { useCallback, useState } from 'react'

import { useSessionConversationCache } from '#components/session-conversation-cache'
import { clearSessionAttachments } from '#components/use-composer-message'
import type {
  SessionOperation,
  SessionOperationError,
} from '#session-operations'
import type { Session } from '#types'

const KEEP_WORKTREE_ERROR_CODES = new Set([
  'unmerged_worktree',
  'dirty_worktree',
  'worktree_process_guard',
])

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
  const confirmationSession = sessions.find(
    (session) => session.id === confirmationSessionId,
  )

  const closeNonDoneSession = useCallback(
    (sessionId: string, keepWorktree: boolean) => {
      void runSessionOperation({
        type: 'close',
        sessionId,
        keepWorktree,
        stopAndClose: true,
      }).then((result) => {
        result.match(
          () => {
            sessionConversationCache?.clearCachedConversation(sessionId)
            clearSessionAttachments(sessionId)
            setError(null)
            setCloseRetrySessionId(null)
          },
          (operationError) => {
            setError(operationError.message)
            setCloseRetrySessionId(
              KEEP_WORKTREE_ERROR_CODES.has(operationError.code ?? '')
                ? sessionId
                : null,
            )
          },
        )
      })
    },
    [
      runSessionOperation,
      sessionConversationCache,
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
