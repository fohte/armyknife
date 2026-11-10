import { setSessionToday } from '#session-actions'
import type { SessionView } from '#session-navigation'
import type { Session } from '#types'

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
    if (
      sessions.some(
        (session) => session.id === sessionId && session.category === 'later',
      )
    ) {
      setLaterExpanded(true)
    }
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
