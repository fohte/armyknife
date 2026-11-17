import { useCallback } from 'react'

import type { HistoryMode, SessionHistoryState } from '#session-history'

type NewSessionMode = 'form' | 'palette'

type NewSessionNavigationOptions = {
  backFromNewSession: () => void
  closeMobileNav: () => void
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
  replaceNewSessionWithSession: (
    sessionId: string,
    conversationOpen: boolean,
  ) => void
  resetNewSession: () => void
  setCommandPaletteOpen: (isOpen: boolean) => void
  setPendingLaunchId: (sessionId: string | null) => void
}

export function useNewSessionNavigation({
  backFromNewSession,
  closeMobileNav,
  navigate,
  replaceNewSessionWithSession,
  resetNewSession,
  setCommandPaletteOpen,
  setPendingLaunchId,
}: NewSessionNavigationOptions) {
  const openNewSession = useCallback(
    (mode: NewSessionMode) => {
      resetNewSession()
      setPendingLaunchId(null)
      setCommandPaletteOpen(false)
      closeMobileNav()
      navigate((current) => ({
        ...current,
        ...(mode === 'form'
          ? {
              view: 'inbox' as const,
              selectedId: null,
              conversationOpen: false,
            }
          : {}),
        newSessionMode: mode,
      }))
    },
    [
      closeMobileNav,
      navigate,
      resetNewSession,
      setCommandPaletteOpen,
      setPendingLaunchId,
    ],
  )
  const closeNewSession = useCallback(() => {
    resetNewSession()
    setPendingLaunchId(null)
    backFromNewSession()
  }, [backFromNewSession, resetNewSession, setPendingLaunchId])
  const switchFromNewSessionToSession = useCallback(
    (sessionId: string, conversationOpen: boolean) => {
      resetNewSession()
      setPendingLaunchId(null)
      replaceNewSessionWithSession(sessionId, conversationOpen)
    },
    [replaceNewSessionWithSession, resetNewSession, setPendingLaunchId],
  )

  return { closeNewSession, openNewSession, switchFromNewSessionToSession }
}
