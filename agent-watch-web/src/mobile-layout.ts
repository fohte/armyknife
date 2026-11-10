import { useEffect, useState, useSyncExternalStore } from 'react'

import { setSessionToday } from '#session-actions'
import type { SessionView } from '#session-navigation'
import type { Session } from '#types'

const mobileBreakpoint = '(max-width: 760px)'

export function useMobileLayout() {
  const isMobile = useSyncExternalStore(
    (listener) => {
      const query = window.matchMedia(mobileBreakpoint)
      query.addEventListener('change', listener)
      return () => {
        query.removeEventListener('change', listener)
      }
    },
    () => window.matchMedia(mobileBreakpoint).matches,
    () => false,
  )
  const [conversationOpen, setConversationOpen] = useState(false)
  const [inProgressExpanded, setInProgressExpanded] = useState(false)
  const [doneExpanded, setDoneExpanded] = useState(false)

  useEffect(() => {
    const viewport = window.visualViewport
    if (!isMobile || viewport === null) {
      document.documentElement.style.removeProperty('--mobile-viewport-height')
      return
    }

    const updateHeight = () => {
      document.documentElement.style.setProperty(
        '--mobile-viewport-height',
        `${String(viewport.height)}px`,
      )
    }
    updateHeight()
    viewport.addEventListener('resize', updateHeight)
    return () => {
      viewport.removeEventListener('resize', updateHeight)
      document.documentElement.style.removeProperty('--mobile-viewport-height')
    }
  }, [isMobile])

  return {
    isMobile,
    conversationOpen,
    setConversationOpen,
    inProgressExpanded,
    setInProgressExpanded,
    doneExpanded,
    setDoneExpanded,
  }
}

type MobileNavigationOptions = {
  isMobile: boolean
  conversationOpen: boolean
  navigableSessions: readonly Session[]
  selectedId: string | null
  selectedSessionExists: boolean
  setSelectedId: (id: string | null) => void
  setConversationOpen: (open: boolean) => void
}

export function useMobileNavigation({
  isMobile,
  conversationOpen,
  navigableSessions,
  selectedId,
  selectedSessionExists,
  setSelectedId,
  setConversationOpen,
}: MobileNavigationOptions) {
  const selectedIsNavigable = navigableSessions.some(
    (session) => session.id === selectedId,
  )

  useEffect(() => {
    if (isMobile && conversationOpen && !selectedIsNavigable) {
      setConversationOpen(false)
    }
    if (!selectedIsNavigable) {
      setSelectedId(navigableSessions[0]?.id ?? null)
    }
  }, [
    conversationOpen,
    isMobile,
    navigableSessions,
    selectedId,
    selectedIsNavigable,
    setConversationOpen,
    setSelectedId,
  ])

  return {
    showConversation:
      isMobile &&
      conversationOpen &&
      selectedIsNavigable &&
      selectedSessionExists,
  }
}

type SessionNavigationOptions = {
  sessions: readonly Session[]
  setSelectedId: (id: string | null) => void
  setView: (view: SessionView) => void
  setConversationOpen: (open: boolean) => void
  setLaterExpanded: (expanded: boolean) => void
  setError: (error: string | null) => void
}

export function createSessionNavigationActions({
  sessions,
  setSelectedId,
  setView,
  setConversationOpen,
  setLaterExpanded,
  setError,
}: SessionNavigationOptions) {
  const openConversation = (sessionId: string) => {
    setSelectedId(sessionId)
    setConversationOpen(true)
  }
  const openSessionInInbox = (sessionId: string) => {
    setView('inbox')
    openConversation(sessionId)
  }
  const openTodaySession = (sessionId: string) => {
    if (
      sessions.some(
        (session) => session.id === sessionId && session.category === 'later',
      )
    ) {
      setLaterExpanded(true)
    }
    openConversation(sessionId)
  }
  const toggleToday = (id: string, isToday: boolean) => {
    void setSessionToday(id, isToday).match(
      () => {
        setError(null)
      },
      () => {
        setError('Could not update TODAY.')
      },
    )
  }

  return {
    openConversation,
    openSessionInInbox,
    openTodaySession,
    toggleToday,
  }
}
