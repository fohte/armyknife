import { setSessionToday } from '#session-actions'
import type { HistoryMode, SessionHistoryState } from '#session-history'
import type { Session } from '#types'

type SessionNavigationOptions = {
  sessions: readonly Session[]
  navigate: (
    update: (current: SessionHistoryState) => SessionHistoryState,
    mode?: HistoryMode,
  ) => void
  setLaterExpanded: (expanded: boolean) => void
  setError: (error: string | null) => void
}

export function createSessionNavigationActions({
  sessions,
  navigate,
  setLaterExpanded,
  setError,
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
