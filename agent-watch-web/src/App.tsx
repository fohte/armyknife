import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { buildBoardProjects, buildWorkTypeNavGroups } from '#board-model'
import { useCommandPaletteController } from '#command-palette-controller'
import { AgentWatchNav } from '#components/agent-watch-nav'
import { BoardView } from '#components/board-view'
import { CommandPalette } from '#components/command-palette'
import { InboxConversationPanes } from '#components/inbox-conversation-panes'
import { LaterMenu } from '#components/later-menu'
import { ResizableColumns } from '#components/resizable-columns'
import {
  filterInboxSessions,
  formatInboxFilterQuery,
  parseInboxFilterQuery,
} from '#inbox-filter'
import { useMobileLayout, useSessionMobileNavigation } from '#mobile-layout'
import {
  getLaterPresetTimes,
  setSessionBackNow,
  setSessionLater,
} from '#session-actions'
import { startSessionFeed } from '#session-feed'
import { useSessionHistory } from '#session-history'
import { useSessionKeyboardNavigation } from '#session-keyboard-navigation'
import { type LaterPreset, type LaterPresetTimes } from '#session-navigation'
import type { Session } from '#types'
import { useSavedInboxViews } from '#use-saved-inbox-views'

export function App() {
  useEffect(() => {
    const preventFileNavigation = (event: DragEvent) => {
      if (!Array.from(event.dataTransfer?.types ?? []).includes('Files')) {
        return
      }
      event.preventDefault()
    }

    window.addEventListener('dragover', preventFileNavigation, true)
    window.addEventListener('drop', preventFileNavigation, true)
    return () => {
      window.removeEventListener('dragover', preventFileNavigation, true)
      window.removeEventListener('drop', preventFileNavigation, true)
    }
  }, [])

  const {
    isMobile,
    inProgressExpanded,
    setInProgressExpanded,
    doneExpanded,
    setDoneExpanded,
  } = useMobileLayout()
  const [sessions, setSessions] = useState<Session[]>([])
  const [workTypeNavLabels, setWorkTypeNavLabels] = useState<
    Record<string, string>
  >({})
  const [sessionsRevision, setSessionsRevision] = useState(0)
  const [sessionsReceivedAt, setSessionsReceivedAt] = useState(() => Date.now())
  const [nowMs, setNowMs] = useState(() => Date.now())
  const {
    selectedId,
    view,
    conversationOpen: mobileConversationOpen,
    backToInbox,
    navigate,
  } = useSessionHistory()
  const [hideQuietProjects, setHideQuietProjects] = useState(true)
  const [focusedProjectName, setFocusedProjectName] = useState<string | null>(
    null,
  )
  const [inboxFilterQuery, setInboxFilterQuery] = useState('')
  const [statusPanelOpen, setStatusPanelOpen] = useState(false)
  const [manualInboxCollapsed, setManualInboxCollapsed] = useState<
    boolean | null
  >(null)
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
  const inboxCollapsed = manualInboxCollapsed ?? statusPanelOpen
  const toggleInbox = useCallback(() => {
    setManualInboxCollapsed((manual) => !(manual ?? statusPanelOpen))
  }, [statusPanelOpen])
  const toggleStatusPanel = useCallback(() => {
    setStatusPanelOpen((open) => !open)
  }, [])

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
  const todaySessions = useMemo(
    () => sessionsWithElapsedTime.filter((session) => session.isToday === true),
    [sessionsWithElapsedTime],
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
  const inboxFilters = useMemo(
    () => parseInboxFilterQuery(inboxFilterQuery),
    [inboxFilterQuery],
  )
  const filteredOrderedSessions = useMemo(
    () => filterInboxSessions(orderedSessions, inboxFilters),
    [inboxFilters, orderedSessions],
  )
  const normalizedInboxFilterQuery = formatInboxFilterQuery(inboxFilters)
  const savedViews = useSavedInboxViews(
    sessionsWithElapsedTime,
    normalizedInboxFilterQuery,
    view === 'inbox',
    setError,
  )
  const navigableSessions = useMemo(
    () =>
      filteredOrderedSessions.filter(
        (session) => laterExpanded || session.category !== 'later',
      ),
    [filteredOrderedSessions, laterExpanded],
  )
  const {
    isOpen: commandPaletteOpen,
    setIsOpen: setCommandPaletteOpen,
    recentSessionIds,
    runCommand: runPaletteCommand,
    openConversation,
    openSessionInInbox,
    openTodaySession,
    toggleToday,
  } = useCommandPaletteController({
    sessions: sessionsWithElapsedTime,
    inboxFilterQuery,
    setInboxFilterQuery,
    navigate,
    setError,
    setFocusedProjectName,
    setLaterExpanded,
    selectedId,
    selectedSessionExists:
      selectedId !== null &&
      sessionsWithElapsedTime.some((session) => session.id === selectedId),
    view,
    isMobile,
    conversationOpen: mobileConversationOpen,
  })
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
          if (isMobile) {
            setLaterExpanded(false)
            navigate(
              (current) => ({
                ...current,
                selectedId:
                  navigableSessions.find((session) => session.id !== sessionId)
                    ?.id ?? null,
                conversationOpen: false,
              }),
              'push',
            )
          }
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
      isMobile,
      laterTimes,
      navigableSessions,
      navigate,
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

  useSessionKeyboardNavigation({
    isMobile,
    laterSessionId,
    laterSubmissionInFlight,
    closeLater,
    submitLater,
    navigableSessions,
    openLater,
    selectedId,
    view,
    navigate,
    toggleInbox,
    toggleStatusPanel,
  })

  const needsActionCount = filteredOrderedSessions.filter(
    (session) => session.category === 'needs_action',
  ).length
  const laterSession = sessionsWithElapsedTime.find(
    (session) => session.id === laterSessionId,
  )
  const selectedSession = sessionsWithElapsedTime.find(
    (session) => session.id === selectedId,
  )
  useEffect(() => {
    if (selectedSession?.category === 'later') {
      setLaterExpanded(true)
    }
  }, [selectedSession?.category])
  const selectInboxSavedView = useCallback(
    (id: string) => {
      const query = savedViews.selectView(id)
      if (query === null) return
      setInboxFilterQuery(query)
      navigate((current) => ({ ...current, view: 'inbox' }))
    },
    [navigate, savedViews.selectView],
  )
  const { showConversation: showMobileConversation } =
    useSessionMobileNavigation({
      isMobile,
      conversationOpen: mobileConversationOpen,
      navigableSessions,
      selectedSessionExists: selectedSession !== undefined,
      sessionsLoaded: loaded,
      navigate,
    })
  const inboxProps = {
    error: error ?? savedViews.storageError,
    loaded,
    selectedId,
    sessions: sessionsWithElapsedTime,
    filterQuery: inboxFilterQuery,
    onFilterQueryChange: setInboxFilterQuery,
    saveViewOpen: savedViews.saveViewOpen,
    saveViewName: savedViews.saveViewName,
    onSaveViewOpenChange: savedViews.setSaveViewOpen,
    onSaveViewNameChange: savedViews.setSaveViewName,
    onSaveView: savedViews.saveView,
    savedViews: savedViews.savedViewsForNav,
    selectedSavedViewId: savedViews.activeSavedViewId,
    onSelectSavedView: selectInboxSavedView,
    onDeleteSavedView: savedViews.deleteView,
    showAllInProgress,
    onShowMore: () => {
      setShowAllInProgress(true)
    },
    onSelect: openConversation,
    laterExpanded,
    onToggleLater: () => {
      setLaterExpanded((expanded) => !expanded)
    },
    onLater: openLater,
    onToggleToday: toggleToday,
    todaySessions,
    onSelectToday: openTodaySession,
    inProgressExpanded,
    onToggleInProgress: () => {
      setInProgressExpanded((expanded) => !expanded)
    },
    doneExpanded,
    onToggleDone: () => {
      setDoneExpanded((expanded) => !expanded)
    },
  }
  const navigation = !isMobile ? (
    <AgentWatchNav
      activeView={view}
      needsActionCount={needsActionCount}
      savedViews={savedViews.savedViewsForNav}
      selectedSavedViewId={savedViews.activeSavedViewId}
      todaySessions={todaySessions}
      workTypeNavGroups={workTypeNavGroups}
      onViewChange={(nextView) => {
        navigate((current) => ({ ...current, view: nextView }))
      }}
      onSelectSavedView={selectInboxSavedView}
      onDeleteSavedView={savedViews.deleteView}
      onSelectToday={openSessionInInbox}
      onSelectSession={openSessionInInbox}
      onOpenPalette={() => {
        setCommandPaletteOpen(true)
      }}
      inboxCollapsed={inboxCollapsed}
      onToggleInbox={toggleInbox}
    />
  ) : null
  const main = (
    <main className="agent-watch-main">
      {view === 'inbox' ? (
        <InboxConversationPanes
          inboxProps={inboxProps}
          sessions={sessionsWithElapsedTime}
          selectedSession={selectedSession}
          refreshKey={sessionsRevision}
          isMobile={isMobile}
          showMobileConversation={showMobileConversation}
          inboxCollapsed={inboxCollapsed}
          statusPanelOpen={statusPanelOpen}
          onToggleStatusPanel={toggleStatusPanel}
          onCloseStatusPanel={() => {
            setStatusPanelOpen(false)
          }}
          onBack={backToInbox}
          onLater={openLater}
          onSelectSession={(id) => {
            if (orderedSessions.some((session) => session.id === id)) {
              navigate((current) => ({ ...current, selectedId: id }))
            }
          }}
        />
      ) : (
        <BoardView
          error={error}
          hideQuietProjects={hideQuietProjects}
          loaded={loaded}
          onHideQuietProjectsChange={setHideQuietProjects}
          onSelectSession={openSessionInInbox}
          projects={projects}
          focusedProjectName={focusedProjectName}
        />
      )}
    </main>
  )
  return (
    <div
      className="agent-watch-shell"
      data-layout={isMobile ? 'mobile' : 'desktop'}
      data-mobile-screen={showMobileConversation ? 'conversation' : 'inbox'}
    >
      {isMobile ? (
        main
      ) : (
        <ResizableColumns
          first={navigation}
          second={main}
          storageKey="agent-watch:navigation-width"
          firstSize={240}
          firstMinSize={160}
          secondMinSize={508}
          label="Resize navigation and content"
        />
      )}
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
      {commandPaletteOpen ? (
        <CommandPalette
          inboxFilterQuery={inboxFilterQuery}
          onClose={() => {
            setCommandPaletteOpen(false)
          }}
          onOpenProject={(project) => {
            setCommandPaletteOpen(false)
            setFocusedProjectName(project)
            navigate((current) => ({
              ...current,
              selectedId: null,
              view: 'board',
              conversationOpen: false,
            }))
          }}
          onOpenSession={(id) => {
            setCommandPaletteOpen(false)
            openSessionInInbox(id)
          }}
          onRunCommand={(command) => {
            void runPaletteCommand(command)
          }}
          recentSessionIds={recentSessionIds}
          sessions={sessionsWithElapsedTime}
        />
      ) : null}
    </div>
  )
}
