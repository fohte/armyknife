import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { buildBoardProjects, buildWorkTypeNavGroups } from '#board-model'
import { AgentWatchNav } from '#components/agent-watch-nav'
import { BoardView } from '#components/board-view'
import { ConversationPane } from '#components/conversation-pane'
import { InboxView } from '#components/inbox-view'
import { LaterMenu } from '#components/later-menu'
import {
  getLaterPresetTimes,
  setSessionBackNow,
  setSessionLater,
  setSessionToday,
} from '#session-actions'
import { startSessionFeed } from '#session-feed'
import {
  getLaterPresetShortcut,
  getSessionShortcutAction,
  type LaterPreset,
  type LaterPresetTimes,
  moveSelection,
  type SessionView,
} from '#session-navigation'
import type { Session } from '#types'

export function App() {
  const [sessions, setSessions] = useState<Session[]>([])
  const [workTypeNavLabels, setWorkTypeNavLabels] = useState<
    Record<string, string>
  >({})
  const [sessionsRevision, setSessionsRevision] = useState(0)
  const [sessionsReceivedAt, setSessionsReceivedAt] = useState(() => Date.now())
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [view, setView] = useState<SessionView>('inbox')
  const [hideQuietProjects, setHideQuietProjects] = useState(true)
  const [showAllInProgress, setShowAllInProgress] = useState(false)
  const [laterExpanded, setLaterExpanded] = useState(false)
  const [laterSessionId, setLaterSessionId] = useState<string | null>(null)
  const [laterTimes, setLaterTimes] = useState<LaterPresetTimes | null>(null)
  const [laterTimesError, setLaterTimesError] = useState(false)
  const [laterMutationPending, setLaterMutationPending] = useState(false)
  const laterSubmissionInFlight = useRef(false)
  const laterRequestId = useRef(0)
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
  const navigableSessions = useMemo(
    () =>
      orderedSessions.filter(
        (session) => laterExpanded || session.category !== 'later',
      ),
    [laterExpanded, orderedSessions],
  )
  const projects = useMemo(
    () => buildBoardProjects(sessionsWithElapsedTime),
    [sessionsWithElapsedTime],
  )
  const workTypeNavGroups = useMemo(
    () => buildWorkTypeNavGroups(sessionsWithElapsedTime, workTypeNavLabels),
    [sessionsWithElapsedTime, workTypeNavLabels],
  )

  const openLater = useCallback((id: string) => {
    const requestId = ++laterRequestId.current
    setLaterTimes(null)
    setLaterTimesError(false)
    setLaterSessionId(id)
    setError(null)
    void getLaterPresetTimes().match(
      (times) => {
        if (requestId === laterRequestId.current) {
          setLaterTimes(times)
        }
      },
      () => {
        if (requestId === laterRequestId.current) {
          setLaterTimesError(true)
          setError('Could not load Later return times.')
        }
      },
    )
  }, [])
  const closeLater = useCallback(() => {
    laterRequestId.current += 1
    setLaterSessionId(null)
  }, [])
  const beginLaterMutation = useCallback(() => {
    if (laterSubmissionInFlight.current) {
      return false
    }
    laterSubmissionInFlight.current = true
    setLaterMutationPending(true)
    return true
  }, [])
  const finishLaterMutation = useCallback(() => {
    laterSubmissionInFlight.current = false
    setLaterMutationPending(false)
  }, [])
  const submitLater = useCallback(
    (sessionId: string, preset: LaterPreset) => {
      const expectedDeadline = laterTimes?.[preset].deadline
      if (expectedDeadline === undefined) {
        return
      }
      if (!beginLaterMutation()) {
        return
      }

      void setSessionLater(sessionId, preset, expectedDeadline).match(
        () => {
          finishLaterMutation()
          closeLater()
          setError(null)
        },
        () => {
          finishLaterMutation()
          openLater(sessionId)
          setError('Could not move the session to Later.')
        },
      )
    },
    [
      beginLaterMutation,
      closeLater,
      finishLaterMutation,
      laterTimes,
      openLater,
    ],
  )
  const submitBackNow = useCallback(
    (sessionId: string) => {
      if (!beginLaterMutation()) {
        return
      }
      void setSessionBackNow(sessionId).match(
        () => {
          finishLaterMutation()
          closeLater()
          setError(null)
        },
        () => {
          finishLaterMutation()
          setError('Could not return the session now.')
        },
      )
    },
    [beginLaterMutation, closeLater, finishLaterMutation],
  )

  useEffect(
    () =>
      startSessionFeed((response) => {
        const receivedAt = Date.now()
        setWorkTypeNavLabels(response.workTypeNavLabels ?? {})
        setSessions(
          response.sessions.filter(
            (session) =>
              session.category === 'needs_action' ||
              session.category === 'later' ||
              session.category === 'in_progress' ||
              session.category === 'done',
          ),
        )
        setSessionsRevision((revision) => revision + 1)
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
    if (!navigableSessions.some((session) => session.id === selectedId)) {
      setSelectedId(navigableSessions[0]?.id ?? null)
    }
  }, [navigableSessions, selectedId])

  useEffect(() => {
    const hiddenProgress = orderedSessions.filter(
      (session) => session.category === 'in_progress',
    )
    if (hiddenProgress.slice(2).some((session) => session.id === selectedId)) {
      setShowAllInProgress(true)
    }
  }, [orderedSessions, selectedId])

  useEffect(() => {
    if (
      laterSessionId !== null &&
      !sessions.some((session) => session.id === laterSessionId)
    ) {
      closeLater()
    }
  }, [closeLater, laterSessionId, sessions])

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
          if (!laterSubmissionInFlight.current) {
            closeLater()
          }
          return
        }
        const preset = getLaterPresetShortcut(event.key)
        if (preset !== null) {
          event.preventDefault()
          submitLater(laterSessionId, preset)
        }
        return
      }

      const action = getSessionShortcutAction(event.key, event.shiftKey, view)
      if (action?.type === 'view') {
        event.preventDefault()
        setView(action.view)
      } else if (action?.type === 'move' && view === 'inbox') {
        event.preventDefault()
        setSelectedId((current) =>
          moveSelection(navigableSessions, current, action.key),
        )
      } else if (
        action?.type === 'later' &&
        selectedId !== null &&
        view === 'inbox'
      ) {
        event.preventDefault()
        openLater(selectedId)
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [
    closeLater,
    laterSessionId,
    navigableSessions,
    openLater,
    selectedId,
    submitLater,
    view,
  ])

  const needsActionCount = sessionsWithElapsedTime.filter(
    (session) => session.category === 'needs_action',
  ).length
  const laterSession = sessionsWithElapsedTime.find(
    (session) => session.id === laterSessionId,
  )
  const selectedSession = sessionsWithElapsedTime.find(
    (session) => session.id === selectedId,
  )

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
  const openSessionInInbox = (sessionId: string) => {
    setSelectedId(sessionId)
    setView('inbox')
  }

  return (
    <div className="agent-watch-shell">
      <AgentWatchNav
        activeView={view}
        needsActionCount={needsActionCount}
        todaySessions={sessionsWithElapsedTime.filter(
          (session) => session.isToday === true,
        )}
        workTypeNavGroups={workTypeNavGroups}
        onViewChange={setView}
        onSelectToday={(id) => {
          setView('inbox')
          setSelectedId(id)
        }}
        onSelectSession={openSessionInInbox}
      />
      <main className="agent-watch-main">
        {view === 'inbox' ? (
          <>
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
            {selectedSession ? (
              <ConversationPane
                key={selectedSession.id}
                session={selectedSession}
                refreshKey={sessionsRevision}
                onSelectSession={(id) => {
                  if (orderedSessions.some((session) => session.id === id)) {
                    setSelectedId(id)
                  }
                }}
              />
            ) : null}
          </>
        ) : (
          <BoardView
            error={error}
            hideQuietProjects={hideQuietProjects}
            loaded={loaded}
            onHideQuietProjectsChange={setHideQuietProjects}
            onSelectSession={openSessionInInbox}
            projects={projects}
          />
        )}
      </main>
      {laterSession !== undefined ? (
        <LaterMenu
          sessionTitle={laterSession.title}
          isLater={laterSession.category === 'later'}
          laterTimes={laterTimes}
          laterTimesError={laterTimesError}
          isSubmitting={laterMutationPending}
          onClose={() => {
            if (!laterSubmissionInFlight.current) {
              closeLater()
            }
          }}
          onBackNow={() => {
            submitBackNow(laterSession.id)
          }}
          onSelect={(preset) => {
            submitLater(laterSession.id, preset)
          }}
        />
      ) : null}
    </div>
  )
}
