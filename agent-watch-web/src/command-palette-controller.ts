import { useCallback, useEffect, useRef, useState } from 'react'

import {
  getRecentSessionStorage,
  type PaletteItem,
  readRecentSessionIds,
  recordRecentSessionId,
} from '#command-palette-model'
import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { archiveSession, openSessionInTerminal } from '#session-actions'
import type { HistoryMode, SessionHistoryState } from '#session-history'
import { createSessionNavigationActions } from '#session-navigation-actions'
import type { Session } from '#types'

type Navigate = (
  update: (current: SessionHistoryState) => SessionHistoryState,
  mode?: HistoryMode,
) => void

type CommandPaletteControllerOptions = {
  sessions: readonly Session[]
  navigate: Navigate
  setError: (error: string | null) => void
  setFocusedProjectName: (project: string | null) => void
  setLaterExpanded: (expanded: boolean) => void
  selectedId: string | null
  selectedSessionExists: boolean
  view: SessionHistoryState['view']
  isMobile: boolean
  conversationOpen: boolean
}

export function useCommandPaletteController({
  sessions,
  navigate,
  setError,
  setFocusedProjectName,
  setLaterExpanded,
  selectedId,
  selectedSessionExists,
  view,
  isMobile,
  conversationOpen,
}: CommandPaletteControllerOptions) {
  const [isOpen, setIsOpen] = useState(false)
  const [recentSessionIds, setRecentSessionIds] = useState(() =>
    readRecentSessionIds(getRecentSessionStorage()),
  )
  const archiveBatchInFlight = useRef(false)
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
    setError,
  })
  const openConversation = withRecentSession(openConversationAction)
  const openSessionInInbox = withRecentSession(openSessionInInboxAction)
  const openTodaySession = withRecentSession(openTodaySessionAction)
  const focusSession = useCallback(
    (id: string) => {
      void openSessionInTerminal(id).match(
        () => {
          setError(null)
        },
        () => {
          setError('Could not open the session in terminal.')
        },
      )
    },
    [setError],
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
        case 'archive-done': {
          if (archiveBatchInFlight.current) return
          archiveBatchInFlight.current = true
          const doneSessions = sessions.filter(
            (session) => session.category === 'done',
          )
          let failure: string | null = null
          for (const session of doneSessions) {
            const archived = await archiveSession(session.id).match(
              () => true,
              () => false,
            )
            if (!archived) {
              failure = `Could not archive ${session.title}.`
              break
            }
          }
          archiveBatchInFlight.current = false
          setError(failure)
          return
        }
        default:
          return
      }
    },
    [navigate, sessions, setError, setFocusedProjectName],
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
    toggleToday,
    runCommand,
  }
}
