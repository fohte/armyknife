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
  handOffSessionLaunch: () => void
  requestComposerFocus: (sessionId: string) => void
}

export function useNewSessionNavigation({
  backFromNewSession,
  closeMobileNav,
  navigate,
  replaceNewSessionWithSession,
  resetNewSession,
  setCommandPaletteOpen,
  handOffSessionLaunch,
  requestComposerFocus,
}: NewSessionNavigationOptions) {
  const openNewSession = useCallback(
    (mode: NewSessionMode) => {
      handOffSessionLaunch()
      resetNewSession()
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
      handOffSessionLaunch,
      navigate,
      resetNewSession,
      setCommandPaletteOpen,
    ],
  )
  const closeNewSession = useCallback(() => {
    resetNewSession()
    backFromNewSession()
  }, [backFromNewSession, resetNewSession])
  const switchFromNewSessionToSession = useCallback(
    (sessionId: string, conversationOpen: boolean) => {
      resetNewSession()
      replaceNewSessionWithSession(sessionId, conversationOpen)
      requestComposerFocus(sessionId)
    },
    [replaceNewSessionWithSession, requestComposerFocus, resetNewSession],
  )

  return { closeNewSession, openNewSession, switchFromNewSessionToSession }
}
