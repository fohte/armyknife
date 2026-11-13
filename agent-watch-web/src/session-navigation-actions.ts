import type { HistoryMode, SessionHistoryState } from '#session-history'
import type { Session } from '#types'

type SessionNavigationOptions = {
  sessions: readonly Session[]
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
  setLaterExpanded: (expanded: boolean) => void
  toggleToday: (id: string, isToday: boolean) => void
}

export function createSessionNavigationActions({
  sessions,
  navigate,
  setLaterExpanded,
  toggleToday,
}: SessionNavigationOptions) {
  const openConversation = (sessionId: string) => {
    navigate((current) => ({
      ...current,
      selectedId: sessionId,
      conversationOpen: true,
    }))
  }
  const openSessionInInbox = (sessionId: string) => {
    if (
      sessions.some(
        (session) => session.id === sessionId && session.category === 'later',
      )
    ) {
      setLaterExpanded(true)
    }
    navigate((current) => ({
      ...current,
      selectedId: sessionId,
      view: 'inbox',
      conversationOpen: true,
    }))
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
    openConversation,
    openSessionInInbox,
    openTodaySession,
    toggleToday,
  }
}
