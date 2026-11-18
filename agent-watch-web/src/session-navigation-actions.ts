import type { HistoryMode, SessionHistoryState } from '#session-history'
import type { Session } from '#types'

type SessionNavigationOptions = {
  sessions: readonly Session[]
  requestComposerFocus: (sessionId: string) => void
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
  setLaterExpanded: (expanded: boolean) => void
  toggleToday: (id: string, isToday: boolean) => void
}

export function createSessionNavigationActions({
  sessions,
  requestComposerFocus,
  navigate,
  setLaterExpanded,
  toggleToday,
}: SessionNavigationOptions) {
  const selectSession = (sessionId: string) => {
    navigate((current) => ({ ...current, selectedId: sessionId }))
    requestComposerFocus(sessionId)
  }
  const openConversation = (sessionId: string) => {
    navigate((current) => ({
      ...current,
      selectedId: sessionId,
      conversationOpen: true,
    }))
    requestComposerFocus(sessionId)
  }
  const openSessionInInbox = (
    sessionId: string,
    mode: HistoryMode = 'push',
  ) => {
    if (
      sessions.some(
        (session) => session.id === sessionId && session.category === 'later',
      )
    ) {
      setLaterExpanded(true)
    }
    navigate(
      (current) => ({
        ...current,
        selectedId: sessionId,
        view: 'inbox',
        conversationOpen: true,
        newSessionMode: null,
      }),
      mode,
    )
    requestComposerFocus(sessionId)
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
  return {
    selectSession,
    openConversation,
    openSessionInInbox,
    openTodaySession,
    toggleToday,
  }
}
