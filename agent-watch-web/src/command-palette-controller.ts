import type { Result } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

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
import type {
  SessionOperation,
  SessionOperationError,
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
  navigate: Navigate
  setError: (error: string | null) => void
  setCloseRetrySessionId: (id: string | null) => void
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
}

export function useCommandPaletteController({
  sessions,
  navigate,
  setError,
  setCloseRetrySessionId,
  setFocusedProjectName,
  setLaterExpanded,
  selectedId,
  selectedSessionExists,
  view,
  isMobile,
  conversationOpen,
  runOperation,
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
  const closeDoneSessions = useCallback(
    async (sessionsToClose: readonly Session[], keepWorktree = false) => {
      const seenSessionIds = new Set<string>()
      const newDoneSessions = sessionsToClose.filter((session) => {
        if (
          session.category !== 'done' ||
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
        let lastFailure: string | null = null
        while (closeQueue.current.length > 0) {
          const batch = closeQueue.current.shift()
          if (batch === undefined) continue

          let failure: string | null = null
          let unmergedSessionId: string | null = null
          for (const session of batch.sessions) {
            const result = await runOperation({
              type: 'close',
              sessionId: session.id,
              keepWorktree: batch.keepWorktree,
            })
            if (result.isErr()) {
              failure =
                result.error.message || `Could not close ${session.title}.`
              if (result.error.code === 'unmerged_worktree') {
                unmergedSessionId = session.id
              }
              break
            }
          }
          for (const session of batch.sessions) {
            pendingCloseSessionIds.current.delete(session.id)
          }
          if (failure !== null) {
            lastFailure = failure
            setError(failure)
            setCloseRetrySessionId(unmergedSessionId)
          } else {
            batch.sessions.forEach(({ id }) => {
              clearSessionAttachments(id)
            })
            if (closeQueue.current.length === 0 && lastFailure === null) {
              setError(null)
              setCloseRetrySessionId(null)
            }
          }
          batch.resolve()
        }
        closeBatchInFlight.current = false
        if (lastFailure !== null) setError(lastFailure)
      }

      await batchComplete
    },
    [
      sessionConversationCache?.clearCachedConversation,
      runOperation,
      setCloseRetrySessionId,
      setError,
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
    openConversation: openConversationAction,
    openSessionInInbox: openSessionInInboxAction,
    openTodaySession: openTodaySessionAction,
    toggleToday,
  } = createSessionNavigationActions({
    sessions,
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
  const openSessionInInbox = withRecentSession(openSessionInInboxAction)
  const openTodaySession = withRecentSession(openTodaySessionAction)
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
        case 'switch-inbox':
          navigate((current) => ({ ...current, view: 'inbox' }))
          return
        case 'close-done': {
          await closeDoneSessions(sessions)
          return
        }
        default:
          return
      }
    },
    [
      closeDoneSessions,
      navigate,
      sessions,
      setCloseRetrySessionId,
      setError,
      setFocusedProjectName,
    ],
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
    openSessionInInbox,
    openTodaySession,
    focusSession,
    pendingFocusSessionId,
    closeDoneSessions,
    toggleToday,
    runCommand,
  }
}
