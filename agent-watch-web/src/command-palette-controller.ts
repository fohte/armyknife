import type { Result } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

import type { StartAsyncActionOptions } from '#async-action-feedback'
import {
  getRecentSessionStorage,
  type PaletteItem,
  readRecentSessionIds,
  recordRecentSessionId,
} from '#command-palette-model'
import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { useSessionConversationCache } from '#components/session-conversation-cache'
import { clearSessionAttachments } from '#components/use-composer-message'
import type { HistoryMode, SessionHistoryState } from '#session-history'
import { createSessionNavigationActions } from '#session-navigation-actions'
import {
  type SessionOperation,
  type SessionOperationError,
} from '#session-operations'
import type { Session } from '#types'

type Navigate = (
  update: (current: SessionHistoryState) => SessionHistoryState,
  mode?: HistoryMode,
) => void

type CloseBatch = {
  sessions: readonly Session[]
  keepWorktree: boolean
  resolve: () => void
}

type CommandPaletteControllerOptions = {
  sessions: readonly Session[]
  requestComposerFocus: (sessionId: string) => void
  navigate: Navigate
  setError: (error: string | null) => void
  setFocusedProjectName: (project: string | null) => void
  setLaterExpanded: (expanded: boolean) => void
  selectedId: string | null
  selectedSessionExists: boolean
  view: SessionHistoryState['view']
  isMobile: boolean
  conversationOpen: boolean
  runOperation: (
    operation: SessionOperation,
  ) => Promise<Result<unknown, SessionOperationError>>
  startAsyncAction?: (options: StartAsyncActionOptions) => number
}

export function useCommandPaletteController({
  sessions,
  requestComposerFocus,
  navigate,
  setError,
  setFocusedProjectName,
  setLaterExpanded,
  selectedId,
  selectedSessionExists,
  view,
  isMobile,
  conversationOpen,
  runOperation,
  startAsyncAction,
}: CommandPaletteControllerOptions) {
  const sessionConversationCache = useSessionConversationCache()
  const [isOpen, setIsOpen] = useState(false)
  const [pendingFocusSessionId, setPendingFocusSessionId] = useState<
    string | null
  >(null)
  const [recentSessionIds, setRecentSessionIds] = useState(() =>
    readRecentSessionIds(getRecentSessionStorage()),
  )
  const closeQueue = useRef<CloseBatch[]>([])
  const pendingCloseSessionIds = useRef(new Set<string>())
  const closeBatchInFlight = useRef(false)
  const focusSession = useCallback(
    async (id: string) => {
      setPendingFocusSessionId(id)
      const result = await runOperation({ type: 'focus', sessionId: id })
      setPendingFocusSessionId(null)
      setError(result.isOk() ? null : 'Could not open the session in terminal.')
      setIsOpen(false)
    },
    [runOperation, setError],
  )
  const closeDoneSessions = useCallback(
    async (sessionsToClose: readonly Session[], keepWorktree = false) => {
      const seenSessionIds = new Set<string>()
      const newDoneSessions = sessionsToClose.filter((session) => {
        if (
          session.category !== 'done' ||
          (session.todoLaterCount ?? 0) > 0 ||
          pendingCloseSessionIds.current.has(session.id) ||
          seenSessionIds.has(session.id)
        ) {
          return false
        }
        seenSessionIds.add(session.id)
        return true
      })
      if (newDoneSessions.length === 0) return

      for (const session of newDoneSessions) {
        sessionConversationCache?.clearCachedConversation(session.id)
        pendingCloseSessionIds.current.add(session.id)
      }

      let resolveBatch: () => void = () => undefined
      const batchComplete = new Promise<void>((resolve) => {
        resolveBatch = resolve
      })
      closeQueue.current.push({
        sessions: newDoneSessions,
        keepWorktree,
        resolve: resolveBatch,
      })

      if (!closeBatchInFlight.current) {
        closeBatchInFlight.current = true
        while (closeQueue.current.length > 0) {
          const batch = closeQueue.current.shift()
          if (batch === undefined) continue

          let failed = false
          for (const session of batch.sessions) {
            const operation = {
              type: 'close',
              sessionId: session.id,
              keepWorktree: batch.keepWorktree,
            } as const
            const resultPromise = runOperation(operation)
            let firstAttempt = true
            startAsyncAction?.({
              kind: 'close',
              sessionId: session.id,
              sessionTitle: session.title,
              run: async () => {
                const isInitialAttempt = firstAttempt
                firstAttempt = false
                const actionResult = await (isInitialAttempt
                  ? resultPromise
                  : runOperation(operation))
                return actionResult.match(
                  () => {
                    if (!isInitialAttempt) {
                      clearSessionAttachments(session.id)
                      setError(null)
                    }
                    return { status: 'closed' as const }
                  },
                  (error) => {
                    const message =
                      error.message || `Could not close ${session.title}.`
                    return {
                      status: 'closeFailed' as const,
                      error: message,
                      ...(error.checks === undefined
                        ? {}
                        : { checks: error.checks }),
                    }
                  },
                )
              },
              onCloseKeepWorktree: () => {
                void closeDoneSessions([session], true)
              },
              onOpenInTerminal: () => {
                void focusSession(session.id)
              },
            })
            const result = await resultPromise
            if (result.isErr()) {
              failed = true
              break
            }
          }
          for (const session of batch.sessions) {
            pendingCloseSessionIds.current.delete(session.id)
          }
          if (!failed) {
            batch.sessions.forEach(({ id }) => {
              clearSessionAttachments(id)
            })
            if (closeQueue.current.length === 0) {
              setError(null)
            }
          }
          batch.resolve()
        }
        closeBatchInFlight.current = false
      }

      await batchComplete
    },
    [
      sessionConversationCache?.clearCachedConversation,
      runOperation,
      startAsyncAction,
      setError,
      focusSession,
    ],
  )
  const recordRecentSession = useCallback((id: string) => {
    recordRecentSessionId(getRecentSessionStorage(), id)
    setRecentSessionIds(readRecentSessionIds(getRecentSessionStorage()))
  }, [])
  const withRecentSession = useCallback(
    (action: (id: string) => void) => (id: string) => {
      recordRecentSession(id)
      action(id)
    },
    [recordRecentSession],
  )
  const {
    selectSession: selectSessionAction,
    openConversation: openConversationAction,
    openSessionInInbox: openSessionInInboxAction,
    openTodaySession: openTodaySessionAction,
    toggleToday,
  } = createSessionNavigationActions({
    sessions,
    requestComposerFocus,
    navigate,
    setLaterExpanded,
    toggleToday: (id, isToday) => {
      void runOperation({ type: 'today', sessionId: id, isToday }).then(
        (result) => {
          setError(result.isOk() ? null : 'Could not update Starred.')
        },
      )
    },
  })
  const openConversation = withRecentSession(openConversationAction)
  const openSessionInInbox = useCallback(
    (id: string, mode?: HistoryMode) => {
      recordRecentSession(id)
      openSessionInInboxAction(id, mode)
    },
    [openSessionInInboxAction, recordRecentSession],
  )
  const openTodaySession = withRecentSession(openTodaySessionAction)
  useEffect(() => {
    if (
      selectedId !== null &&
      selectedSessionExists &&
      view === 'inbox' &&
      (!isMobile || conversationOpen)
    ) {
      recordRecentSession(selectedId)
    }
  }, [
    conversationOpen,
    isMobile,
    recordRecentSession,
    selectedId,
    selectedSessionExists,
    view,
  ])
  const runCommand = useCallback(
    async (command: Extract<PaletteItem, { type: 'command' }>) => {
      setIsOpen(false)
      switch (command.id) {
        case 'switch-board':
          setFocusedProjectName(null)
          navigate((current) => ({ ...current, view: 'board' }))
          return
        case 'close-done': {
          await closeDoneSessions(sessions)
          return
        }
        default:
          return
      }
    },
    [closeDoneSessions, navigate, sessions, setError, setFocusedProjectName],
  )

  useEffect(() => {
    const { modifier } = getCommandPaletteShortcut(navigator.platform)
    const onKeyDown = (event: KeyboardEvent) => {
      if (event[modifier] && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setIsOpen(true)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [])

  return {
    isOpen,
    setIsOpen,
    recentSessionIds,
    openConversation,
    selectSession: selectSessionAction,
    openSessionInInbox,
    openTodaySession,
    focusSession,
    pendingFocusSessionId,
    closeDoneSessions,
    toggleToday,
    runCommand,
  }
}
