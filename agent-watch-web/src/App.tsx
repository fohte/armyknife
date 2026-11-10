import { useEffect, useMemo, useState } from 'react'

import { AgentWatchNav } from '#components/agent-watch-nav'
import { InboxView } from '#components/inbox-view'
import { LaterMenu } from '#components/later-menu'
import { setSessionLater, setSessionToday } from '#session-actions'
import { startSessionFeed } from '#session-feed'
import {
  getLaterPresetShortcut,
  getSessionShortcutAction,
  moveSelection,
} from '#session-navigation'
import type { Session } from '#types'

type View = 'inbox' | 'board'

export function App() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [sessionsReceivedAt, setSessionsReceivedAt] = useState(() => Date.now())
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [view, setView] = useState<View>('inbox')
  const [showAllInProgress, setShowAllInProgress] = useState(false)
  const [laterExpanded, setLaterExpanded] = useState(false)
  const [laterSessionId, setLaterSessionId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loaded, setLoaded] = useState(false)

  const elapsedOffsetSeconds = Math.floor(
    Math.max(0, nowMs - sessionsReceivedAt) / 1_000,
  )
  const sessionsWithElapsedTime = useMemo(
    () =>
      sessions.map((session) => {
        const deadlinePassed =
          session.category === 'later' &&
          session.laterUntil !== null &&
          session.laterUntil !== undefined &&
          Date.parse(session.laterUntil) <= nowMs
        return {
          ...session,
          category: deadlinePassed
            ? (session.categoryWhenNotLater ?? 'needs_action')
            : session.category,
          elapsedSeconds: session.elapsedSeconds + elapsedOffsetSeconds,
        }
      }),
    [elapsedOffsetSeconds, sessions],
  )
  const orderedSessions = useMemo(
    () => [
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'needs_action',
      ),
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'later',
      ),
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'in_progress',
      ),
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'done',
      ),
    ],
    [sessionsWithElapsedTime],
  )

  useEffect(
    () =>
      startSessionFeed((response) => {
        const receivedAt = Date.now()
        setSessions(
          response.sessions.filter(
            (session) =>
              session.category === 'needs_action' ||
              session.category === 'later' ||
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

      if (laterSessionId !== null) {
        if (event.key === 'Escape') {
          event.preventDefault()
          setLaterSessionId(null)
          return
        }
        const preset = getLaterPresetShortcut(event.key)
        if (preset !== null) {
          event.preventDefault()
          void setSessionLater(laterSessionId, preset).match(
            () => {
              setLaterSessionId(null)
              setError(null)
            },
            () => {
              setError('Could not move the session to Later.')
            },
          )
        }
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
      } else if (
        action?.type === 'later' &&
        selectedId !== null &&
        view === 'inbox'
      ) {
        event.preventDefault()
        setLaterSessionId(selectedId)
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [laterSessionId, orderedSessions, selectedId, view])

  const needsActionCount = sessionsWithElapsedTime.filter(
    (session) => session.category === 'needs_action',
  ).length
  const laterSession = sessions.find((session) => session.id === laterSessionId)

  const openLater = (id: string) => {
    setLaterSessionId(id)
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

  return (
    <div className="agent-watch-shell">
      <AgentWatchNav
        activeView={view}
        needsActionCount={needsActionCount}
        todaySessions={sessionsWithElapsedTime.filter(
          (session) => session.isToday === true,
        )}
        onViewChange={setView}
        onSelectToday={(id) => {
          setView('inbox')
          setSelectedId(id)
        }}
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
            laterExpanded={laterExpanded}
            onToggleLater={() => {
              setLaterExpanded((expanded) => !expanded)
            }}
            onLater={openLater}
            onToggleToday={toggleToday}
          />
        ) : (
          <section className="board-placeholder" aria-labelledby="board-title">
            <header className="view-header">
              <h1 id="board-title">Board</h1>
            </header>
          </section>
        )}
      </main>
      {laterSession !== undefined ? (
        <LaterMenu
          sessionTitle={laterSession.title}
          onClose={() => {
            setLaterSessionId(null)
          }}
          onSelect={(preset) => {
            void setSessionLater(laterSession.id, preset).match(
              () => {
                setLaterSessionId(null)
                setError(null)
              },
              () => {
                setError('Could not move the session to Later.')
              },
            )
          }}
        />
      ) : null}
    </div>
  )
}
