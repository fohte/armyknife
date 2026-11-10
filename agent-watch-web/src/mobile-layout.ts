import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'

import type { HistoryMode, SessionHistoryState } from '#session-history'
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
  selectedSessionExists: boolean
  sessionsLoaded: boolean
  setSelectedId: (id: string | null) => void
  setConversationOpen: (open: boolean) => void
}

function useMobileNavigation({
  isMobile,
  conversationOpen,
  navigableSessions,
  selectedSessionExists,
  sessionsLoaded,
  setSelectedId,
  setConversationOpen,
}: MobileNavigationOptions) {
  useEffect(() => {
    if (!sessionsLoaded) {
      return
    }
    if (isMobile && conversationOpen && !selectedSessionExists) {
      setConversationOpen(false)
    }
    if (!selectedSessionExists) {
      setSelectedId(navigableSessions[0]?.id ?? null)
    }
  }, [
    conversationOpen,
    isMobile,
    navigableSessions,
    sessionsLoaded,
    setConversationOpen,
    setSelectedId,
  ])

  return {
    showConversation: isMobile && conversationOpen && selectedSessionExists,
  }
}

type SessionMobileNavigationOptions = Omit<
  MobileNavigationOptions,
  'setSelectedId' | 'setConversationOpen'
> & {
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
}

export function useSessionMobileNavigation({
  navigate,
  ...options
}: SessionMobileNavigationOptions) {
  const setSelectedId = useCallback(
    (id: string | null) => {
      navigate((current) => ({ ...current, selectedId: id }), 'replace')
    },
    [navigate],
  )
  const setConversationOpen = useCallback(
    (open: boolean) => {
      navigate((current) => ({ ...current, conversationOpen: open }), 'replace')
    },
    [navigate],
  )

  return useMobileNavigation({
    ...options,
    selectedSessionExists: options.selectedSessionExists,
    setSelectedId,
    setConversationOpen,
  })
}
