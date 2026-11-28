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
  setCommandPaletteOpen: (isOpen: boolean) => void
  handOffSessionLaunch: () => boolean
  requestComposerFocus: (sessionId: string) => void
}

export function useNewSessionNavigation({
  backFromNewSession,
  closeMobileNav,
  navigate,
  replaceNewSessionWithSession,
  setCommandPaletteOpen,
  handOffSessionLaunch,
  requestComposerFocus,
}: NewSessionNavigationOptions) {
  const openNewSession = useCallback(
    (mode: NewSessionMode) => {
      handOffSessionLaunch()
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
    [closeMobileNav, handOffSessionLaunch, navigate, setCommandPaletteOpen],
  )
  const closeNewSession = useCallback(() => {
    handOffSessionLaunch()
    backFromNewSession()
  }, [backFromNewSession, handOffSessionLaunch])
  const switchFromNewSessionToSession = useCallback(
    (sessionId: string, conversationOpen: boolean) => {
      handOffSessionLaunch()
      replaceNewSessionWithSession(sessionId, conversationOpen)
      requestComposerFocus(sessionId)
    },
    [handOffSessionLaunch, replaceNewSessionWithSession, requestComposerFocus],
  )

  return { closeNewSession, openNewSession, switchFromNewSessionToSession }
}
