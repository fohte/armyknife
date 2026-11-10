import { useCallback, useEffect, useState } from 'react'

import {
  type PaletteItem,
  readRecentSessionIds,
  recordRecentSessionId,
} from '#command-palette-model'
import { formatInboxFilterQuery, parseInboxFilterQuery } from '#inbox-filter'
import { archiveSession } from '#session-actions'
import type { HistoryMode, SessionHistoryState } from '#session-history'
import { createSessionNavigationActions } from '#session-navigation-actions'
import type { Session } from '#types'

type Navigate = (
  update: (current: SessionHistoryState) => SessionHistoryState,
  mode?: HistoryMode,
) => void

type CommandPaletteControllerOptions = {
  sessions: readonly Session[]
  inboxFilterQuery: string
  setInboxFilterQuery: (query: string) => void
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
  inboxFilterQuery,
  setInboxFilterQuery,
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
    readRecentSessionIds(window.localStorage),
  )
  const recordRecentSession = useCallback((id: string) => {
    recordRecentSessionId(window.localStorage, id)
    setRecentSessionIds(readRecentSessionIds(window.localStorage))
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
          const doneSessions = sessions.filter(
            (session) => session.category === 'done',
          )
          for (const session of doneSessions) {
            const archived = await archiveSession(session.id).match(
              () => true,
              () => false,
            )
            if (!archived) {
              setError(`Could not archive ${session.title}.`)
              return
            }
          }
          setError(null)
          return
        }
        case 'clear-inbox-filter':
          setInboxFilterQuery('')
          navigate((current) => ({ ...current, view: 'inbox' }))
          return
        default:
          if (command.taskNumber !== undefined) {
            const filters = parseInboxFilterQuery(inboxFilterQuery).filter(
              (filter) => filter.key !== 'task',
            )
            filters.push({
              key: 'task',
              values: [String(command.taskNumber)],
            })
            setInboxFilterQuery(formatInboxFilterQuery(filters))
            navigate((current) => ({ ...current, view: 'inbox' }))
          }
      }
    },
    [
      inboxFilterQuery,
      navigate,
      sessions,
      setError,
      setFocusedProjectName,
      setInboxFilterQuery,
    ],
  )

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
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
    toggleToday,
    recordRecentSession,
    withRecentSession,
    runCommand,
  }
}
