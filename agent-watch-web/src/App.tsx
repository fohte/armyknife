import { useEffect, useMemo, useState } from 'react'

import { AgentWatchNav } from '#components/agent-watch-nav'
import { InboxView } from '#components/inbox-view'
import { startSessionFeed } from '#session-feed'
import { getSessionShortcutAction, moveSelection } from '#session-navigation'
import type { Session } from '#types'

type View = 'inbox' | 'board'

export function App() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [view, setView] = useState<View>('inbox')
  const [showAllInProgress, setShowAllInProgress] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [loaded, setLoaded] = useState(false)

  const orderedSessions = useMemo(
    () => [
      ...sessions.filter((session) => session.category === 'needs_action'),
      ...sessions.filter((session) => session.category === 'in_progress'),
      ...sessions.filter((session) => session.category === 'done'),
    ],
    [sessions],
  )

  useEffect(
    () =>
      startSessionFeed((response) => {
        setSessions(response.sessions)
        setLoaded(true)
      }, setError),
    [],
  )

  useEffect(() => {
    if (!orderedSessions.some((session) => session.id === selectedId)) {
      setSelectedId(orderedSessions[0]?.id ?? null)
    }
  }, [orderedSessions, selectedId])

  useEffect(() => {
    const hiddenProgress = orderedSessions.filter(
      (session) => session.category === 'in_progress',
    )
    if (hiddenProgress.slice(2).some((session) => session.id === selectedId)) {
      setShowAllInProgress(true)
    }
  }, [orderedSessions, selectedId])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        (event.target instanceof HTMLElement &&
          (event.target.isContentEditable ||
            ['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName)))
      ) {
        return
      }

      const action = getSessionShortcutAction(event.key, event.shiftKey)
      if (action?.type === 'view') {
        event.preventDefault()
        setView(action.view)
      } else if (action?.type === 'move') {
        event.preventDefault()
        setSelectedId((current) =>
          moveSelection(orderedSessions, current, action.key),
        )
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [orderedSessions])

  const needsActionCount = sessions.filter(
    (session) => session.category === 'needs_action',
  ).length

  return (
    <div className="agent-watch-shell">
      <AgentWatchNav
        activeView={view}
        needsActionCount={needsActionCount}
        onViewChange={setView}
      />
      <main className="agent-watch-main">
        {view === 'inbox' ? (
          <InboxView
            error={error}
            loaded={loaded}
            selectedId={selectedId}
            sessions={sessions}
            showAllInProgress={showAllInProgress}
            onShowMore={() => {
              setShowAllInProgress(true)
            }}
            onSelect={setSelectedId}
          />
        ) : (
          <section className="board-placeholder" aria-labelledby="board-title">
            <header className="view-header">
              <h1 id="board-title">Board</h1>
            </header>
          </section>
        )}
      </main>
    </div>
  )
}
