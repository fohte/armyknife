import type { Result } from 'neverthrow'
import { useCallback, useState } from 'react'

import { type AsyncAction, useAsyncActionStarter } from '#async-action-feedback'
import { useSessionConversationCache } from '#components/session-conversation-cache'
import { clearSessionAttachments } from '#components/use-composer-message'
import type { CloseWorktreeChecks } from '#session-operation-types'
import {
  type SessionOperation,
  type SessionOperationError,
} from '#session-operations'
import type { Session } from '#types'

type UseSessionCloseOptions = {
  sessions: readonly Session[]
  runSessionOperation: (
    operation: SessionOperation,
  ) => Promise<Result<unknown, SessionOperationError>>
  closeDoneSession: (sessionId: string, keepWorktree?: boolean) => void
  openInTerminal: (sessionId: string) => void
  setError: (error: string | null) => void
}

type DiscardConfirmationSnapshot = {
  sessionId: string
  sessionName: string
  checks: CloseWorktreeChecks
  previousChecks?: CloseWorktreeChecks
}

export type SessionDiscardConfirmationState = {
  sessionId: string
  sessionName: string
  branchName: string | null
  worktreePath: string
  changes: {
    status: CloseWorktreeChecks['changes'][number]['status']
    path: string
    isNew?: boolean
  }[]
  commits: {
    sha: string
    subject: string
    isNew?: boolean
  }[]
  reviewAgain: boolean
}

function confirmationView(
  snapshot: DiscardConfirmationSnapshot,
): SessionDiscardConfirmationState {
  const previousChecks = snapshot.previousChecks
  const targetChanged =
    previousChecks !== undefined &&
    (previousChecks.path !== snapshot.checks.path ||
      previousChecks.branch !== snapshot.checks.branch)
  const previousChanges = new Set(
    previousChecks?.changes.map(({ status, path }) => `${status}:${path}`) ??
      [],
  )
  const previousCommits = new Set(
    previousChecks?.commits.map(({ sha }) => sha) ?? [],
  )

  return {
    sessionId: snapshot.sessionId,
    sessionName: snapshot.sessionName,
    branchName: snapshot.checks.branch,
    worktreePath: snapshot.checks.path,
    changes: snapshot.checks.changes.map((change) => ({
      ...change,
      ...(previousChecks !== undefined &&
      (targetChanged || !previousChanges.has(`${change.status}:${change.path}`))
        ? { isNew: true }
        : {}),
    })),
    commits: snapshot.checks.commits.map((commit) => ({
      ...commit,
      ...(previousChecks !== undefined &&
      (targetChanged || !previousCommits.has(commit.sha))
        ? { isNew: true }
        : {}),
    })),
    reviewAgain: previousChecks !== undefined,
  }
}

function discardPayload(checks: CloseWorktreeChecks) {
  return {
    path: checks.path,
    branch: checks.branch,
    changes: checks.changes,
    commits: checks.commits.map(({ sha }) => sha),
  }
}

function newItemSummary(
  confirmed: CloseWorktreeChecks,
  current: CloseWorktreeChecks,
) {
  const targetChanged =
    confirmed.path !== current.path || confirmed.branch !== current.branch
  const confirmedChanges = new Set(
    confirmed.changes.map(({ status, path }) => `${status}:${path}`),
  )
  const confirmedCommits = new Set(confirmed.commits.map(({ sha }) => sha))
  const newFileCount = current.changes.filter(
    ({ status, path }) =>
      targetChanged || !confirmedChanges.has(`${status}:${path}`),
  ).length
  const newCommitCount = current.commits.filter(
    ({ sha }) => targetChanged || !confirmedCommits.has(sha),
  ).length
  const items = [
    newFileCount > 0
      ? `${String(newFileCount)} new ${newFileCount === 1 ? 'file' : 'files'}`
      : null,
    newCommitCount > 0
      ? `${String(newCommitCount)} new ${newCommitCount === 1 ? 'commit' : 'commits'}`
      : null,
  ].filter((item) => item !== null)
  return items.length === 0
    ? 'The worktree or branch changed after you confirmed.'
    : `Changed after you confirmed: ${items.join(' and ')}.`
}

function runningProcessSummary(checks: CloseWorktreeChecks | undefined) {
  if (checks === undefined || checks.processes.length === 0) return undefined
  const processes = checks.processes
    .map(({ pid, name }) => `${String(pid)} (${name})`)
    .join(', ')
  return `Running in the worktree: ${processes}. Stop it, then Retry.`
}

export function useSessionClose({
  sessions,
  runSessionOperation,
  closeDoneSession,
  openInTerminal,
  setError,
}: UseSessionCloseOptions) {
  const [confirmationSessionId, setConfirmationSessionId] = useState<
    string | null
  >(null)
  const [discardSnapshot, setDiscardSnapshot] =
    useState<DiscardConfirmationSnapshot | null>(null)
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
        discardable: !keepWorktree,
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
                return { status: 'closed' as const }
              },
              (operationError) => {
                return {
                  status: 'closeFailed' as const,
                  error: operationError.message,
                  ...(operationError.checks === undefined
                    ? {}
                    : { checks: operationError.checks }),
                }
              },
            ),
          ),
        onCloseKeepWorktree: () => {
          closeNonDoneSession(sessionId, true)
        },
        onDiscardChanges: (action) => {
          if (
            action.kind !== 'close' ||
            action.checks === undefined ||
            action.checks.commitInspectionUnavailable
          ) {
            return
          }
          setDiscardSnapshot({
            sessionId: action.sessionId,
            sessionName: action.sessionTitle,
            checks: action.checks,
          })
        },
        onOpenInTerminal: () => {
          openInTerminal(sessionId)
        },
      })
    },
    [
      startAsyncAction,
      runSessionOperation,
      openInTerminal,
      sessionConversationCache,
      sessions,
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

  const cancelDiscardConfirmation = useCallback(() => {
    setDiscardSnapshot(null)
  }, [])

  const confirmDiscardChanges = useCallback(() => {
    if (discardSnapshot === null) return
    const snapshot = discardSnapshot
    setDiscardSnapshot(null)
    startAsyncAction({
      kind: 'discard',
      sessionId: snapshot.sessionId,
      sessionTitle: snapshot.sessionName,
      run: async () =>
        runSessionOperation({
          type: 'close',
          sessionId: snapshot.sessionId,
          keepWorktree: false,
          stopAndClose: true,
          discard: discardPayload(snapshot.checks),
        }).then((result) =>
          result.match(
            () => {
              sessionConversationCache?.clearCachedConversation(
                snapshot.sessionId,
              )
              clearSessionAttachments(snapshot.sessionId)
              setError(null)
              return { status: 'deleted' as const }
            },
            (operationError) => {
              const checks = operationError.checks
              if (
                operationError.code === 'close_checks_changed' &&
                checks !== undefined &&
                !checks.commitInspectionUnavailable
              ) {
                return {
                  status: 'deleteChanged' as const,
                  error: operationError.message,
                  message: newItemSummary(snapshot.checks, checks),
                  checks,
                }
              }
              if (operationError.code === 'worktree_process_guard') {
                const message = runningProcessSummary(checks)
                return {
                  status: 'deleteBlocked' as const,
                  error: operationError.message,
                  message: message ?? operationError.message,
                  ...(checks === undefined ? {} : { checks }),
                }
              }
              return {
                status: 'deleteFailed' as const,
                error: operationError.message,
                ...(checks === undefined ? {} : { checks }),
              }
            },
          ),
        ),
      onCloseKeepWorktree: () => {
        closeNonDoneSession(snapshot.sessionId, true)
      },
      onOpenInTerminal: () => {
        openInTerminal(snapshot.sessionId)
      },
      onReviewDiscardAgain: (action: AsyncAction) => {
        if (
          action.kind !== 'discard' ||
          action.checks === undefined ||
          action.checks.commitInspectionUnavailable
        ) {
          return
        }
        setDiscardSnapshot({
          sessionId: action.sessionId,
          sessionName: action.sessionTitle,
          checks: action.checks,
          previousChecks: snapshot.checks,
        })
      },
    })
  }, [
    closeNonDoneSession,
    discardSnapshot,
    openInTerminal,
    runSessionOperation,
    sessionConversationCache,
    setError,
    startAsyncAction,
  ])

  const discardConfirmation =
    discardSnapshot === null ? undefined : confirmationView(discardSnapshot)

  return {
    confirmationSession,
    discardConfirmation,
    requestCloseSession,
    cancelCloseConfirmation,
    confirmCloseSession,
    cancelDiscardConfirmation,
    confirmDiscardChanges,
  }
}
