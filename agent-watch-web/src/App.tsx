import { useEffect, useMemo, useState } from 'react'

import { buildBoardProjects, sortPmProjects } from '#board-model'
import { AgentWatchNav } from '#components/agent-watch-nav'
import { BoardView } from '#components/board-view'
import { InboxView } from '#components/inbox-view'
import { startSessionFeed } from '#session-feed'
import { getSessionShortcutAction, moveSelection } from '#session-navigation'
import type { Session } from '#types'

type View = 'inbox' | 'board'

export function App() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [sessionsReceivedAt, setSessionsReceivedAt] = useState(() => Date.now())
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [view, setView] = useState<View>('inbox')
  const [hideQuietProjects, setHideQuietProjects] = useState(true)
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
  const elapsedOffsetSeconds = Math.floor(
    Math.max(0, nowMs - sessionsReceivedAt) / 1_000,
  )
  const sessionsWithElapsedTime = useMemo(
    () =>
      sessions.map((session) => ({
        ...session,
        elapsedSeconds: session.elapsedSeconds + elapsedOffsetSeconds,
      })),
    [elapsedOffsetSeconds, sessions],
  )
  const projects = useMemo(
    () => buildBoardProjects(sessionsWithElapsedTime),
    [sessionsWithElapsedTime],
  )
  const pmProjects = useMemo(
    () =>
      sortPmProjects(projects).map(({ name, status, needsActionCount }) => ({
        name,
        status,
        needsActionCount,
      })),
    [projects],
  )

  useEffect(
    () =>
      startSessionFeed((response) => {
        const receivedAt = Date.now()
        setSessions(
          response.sessions.filter(
            (session) =>
              session.category === 'needs_action' ||
              session.category === 'in_progress' ||
              session.category === 'done',
          ),
        )
        setSessionsReceivedAt(receivedAt)
        setNowMs(receivedAt)
        setLoaded(true)
      }, setError),
    [],
  )

  useEffect(() => {
    const interval = window.setInterval(() => {
      setNowMs(Date.now())
    }, 1_000)
    return () => {
      window.clearInterval(interval)
    }
  }, [])

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
    if (view !== 'inbox') {
      return
    }

    document
      .querySelector<HTMLButtonElement>(
        '[data-session-row][aria-current="true"]',
      )
      ?.scrollIntoView({ block: 'nearest' })
  }, [selectedId, showAllInProgress, view])

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
      } else if (action?.type === 'move' && view === 'inbox') {
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
  }, [orderedSessions, view])

  const needsActionCount = sessions.filter(
    (session) => session.category === 'needs_action',
  ).length

  return (
    <div className="agent-watch-shell">
      <AgentWatchNav
        activeView={view}
        needsActionCount={needsActionCount}
        pmProjects={pmProjects}
        onViewChange={setView}
      />
      <main className="agent-watch-main">
        {view === 'inbox' ? (
          <InboxView
            error={error}
            loaded={loaded}
            selectedId={selectedId}
            sessions={sessionsWithElapsedTime}
            showAllInProgress={showAllInProgress}
            onShowMore={() => {
              setShowAllInProgress(true)
            }}
            onSelect={setSelectedId}
          />
        ) : (
          <BoardView
            error={error}
            hideQuietProjects={hideQuietProjects}
            loaded={loaded}
            onHideQuietProjectsChange={setHideQuietProjects}
            projects={projects}
          />
        )}
      </main>
    </div>
  )
}
