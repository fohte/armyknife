import { useCallback, useEffect, useState } from 'react'

import { AppWorkspace } from '#app-workspace'
import {
  AsyncActionFeedbackProvider,
  useAsyncActionStarter,
} from '#async-action-feedback'
import { useCommandPaletteController } from '#command-palette-controller'
import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { AgentUsageProvider } from '#components/agent-usage-context'
import { SessionConversationCacheProvider } from '#components/session-conversation-cache'
import { SessionElapsedStartedAtProvider } from '#components/session-elapsed-context'
import { startFreezeProfiler } from '#freeze-profiler'
import { useMobileLayout, useSessionMobileNavigation } from '#mobile-layout'
import { findLaunchedSession } from '#new-session-navigation'
import { startSessionFeed } from '#session-feed'
import { useSessionHistory } from '#session-history'
import { useSessionKeyboardNavigation } from '#session-keyboard-navigation'
import { SessionOperationProvider } from '#session-operation-context'
import type { Session } from '#types'
import { useAppSessionOperations } from '#use-app-session-operations'
import { useAppSessionProjection } from '#use-app-session-projection'
import { useComposerFocus } from '#use-composer-focus'
import { useInboxSort } from '#use-inbox-sort'
import { useLaterDeadlineClock } from '#use-later-deadline-clock'
import { useLaterSessionActions } from '#use-later-session-actions'
import { useNewSession } from '#use-new-session'
import { useNewSessionNavigation } from '#use-new-session-navigation'
import { usePendingMergeRetryNavigation } from '#use-pending-merge-retry-navigation'
import { useSessionClose } from '#use-session-close'
import { useSessionRefreshRevisions } from '#use-session-refresh-revisions'

export function App() {
  return (
    <AsyncActionFeedbackProvider>
      <SessionOperationProvider>
        <SessionConversationCacheProvider>
          <AgentUsageProvider>
            <AppContent />
          </AgentUsageProvider>
        </SessionConversationCacheProvider>
      </SessionOperationProvider>
    </AsyncActionFeedbackProvider>
  )
}

function AppContent() {
  useEffect(() => startFreezeProfiler(), [])

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
    waitingExpanded,
    setWaitingExpanded,
    doneExpanded,
    setDoneExpanded,
  } = useMobileLayout()
  const [sessions, setSessions] = useState<Session[]>([])
  const {
    pendingOperations,
    optimisticSessions,
    dispatchSessionOperation,
    runSessionOperation,
    recordSessionSnapshot,
  } = useAppSessionOperations(sessions)
  const { nowMs, setNowMs } = useLaterDeadlineClock(optimisticSessions)
  const [workTypeNavLabels, setWorkTypeNavLabels] = useState<
    Record<string, string>
  >({})
  const { sessionRevisions, recordSessionChanges } =
    useSessionRefreshRevisions()
  const [sessionsReceivedAt, setSessionsReceivedAt] = useState(() => Date.now())
  const {
    selectedId,
    view,
    conversationOpen: mobileConversationOpen,
    newSessionMode,
    backFromNewSession,
    backToInbox,
    navigate,
    replaceNewSessionWithSession,
  } = useSessionHistory()
  const {
    composerFocusRequest,
    requestComposerFocus,
    handleComposerFocusRequest,
    focusComposer,
  } = useComposerFocus({ isMobile, selectedId })
  const [hideQuietProjects, setHideQuietProjects] = useState(true)
  const [focusedProjectName, setFocusedProjectName] = useState<string | null>(
    null,
  )
  const [inboxFilterQuery, setInboxFilterQuery] = useState('')
  const [statusPanelOpen, setStatusPanelOpen] = useState(false)
  const [mobileNavOpen, setMobileNavOpen] = useState(false)
  const [pendingLaunchId, setPendingLaunchId] = useState<string | null>(null)
  const [manualInboxCollapsed, setManualInboxCollapsed] = useState<
    boolean | null
  >(null)
  const [showAllInProgress, setShowAllInProgress] = useState(false)
  const [showAllWaiting, setShowAllWaiting] = useState(false)
  const [laterExpanded, setLaterExpanded] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [availableVersion, setAvailableVersion] = useState<string | null>(null)
  const [dismissedVersion, setDismissedVersion] = useState<string | null>(null)
  const startAsyncAction = useAsyncActionStarter((feedbackError) => {
    setError(feedbackError.message)
  })
  const [loaded, setLoaded] = useState(false)
  const newSession = useNewSession(newSessionMode !== null)
  const inboxCollapsed = manualInboxCollapsed ?? statusPanelOpen
  const toggleInbox = useCallback(() => {
    setManualInboxCollapsed((manual) => !(manual ?? statusPanelOpen))
  }, [statusPanelOpen])
  const toggleStatusPanel = useCallback(() => {
    setStatusPanelOpen((open) => !open)
  }, [])

  const {
    sessionsWithElapsedTime,
    todaySessions,
    orderedSessions,
    navigableSessions,
    savedViews,
    projects,
    workTypeNavGroups,
    needsActionCount,
  } = useAppSessionProjection({
    optimisticSessions,
    nowMs,
    inboxFilterQuery,
    laterExpanded,
    workTypeNavLabels,
    view,
    setError,
  })
  const inboxSort = useInboxSort({
    orderedSessions,
    navigableSessions,
    selectedId,
    setShowAllInProgress,
    setShowAllWaiting,
  })
  const {
    laterSessionId,
    laterTimes,
    laterTimesError,
    laterSubmissionInFlight,
    laterMutationPending,
    openLater,
    closeLater,
    submitLater,
    submitBackNow,
  } = useLaterSessionActions({
    isMobile,
    sessions,
    navigableSessions: inboxSort.navigableSessions,
    pendingOperations,
    navigate,
    runSessionOperation,
    setError,
    setLaterExpanded,
  })
  const {
    isOpen: commandPaletteOpen,
    setIsOpen: setCommandPaletteOpen,
    recentSessionIds,
    runCommand: runPaletteCommand,
    selectSession,
    openConversation,
    openSessionInInbox,
    openTodaySession,
    focusSession,
    pendingFocusSessionId,
    closeDoneSessions,
    toggleToday,
  } = useCommandPaletteController({
    sessions: sessionsWithElapsedTime,
    requestComposerFocus,
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
    runOperation: runSessionOperation,
    startAsyncAction,
  })

  const closeDoneSession = useCallback(
    (sessionId: string, keepWorktree = false) => {
      const session = sessionsWithElapsedTime.find(
        (candidate) => candidate.id === sessionId,
      )
      if (session !== undefined) {
        void closeDoneSessions([session], keepWorktree)
      }
    },
    [closeDoneSessions, sessionsWithElapsedTime],
  )
  const {
    confirmationSession: closeConfirmationSession,
    requestCloseSession,
    cancelCloseConfirmation,
    confirmCloseSession,
  } = useSessionClose({
    sessions: sessionsWithElapsedTime,
    runSessionOperation,
    closeDoneSession,
    openInTerminal: (sessionId) => {
      void focusSession(sessionId)
    },
    setError,
  })

  useEffect(
    () =>
      startSessionFeed(
        (response, changedSessionIds) => {
          const receivedAt = Date.now()
          recordSessionSnapshot(response.sessions)
          setWorkTypeNavLabels(response.workTypeNavLabels ?? {})
          setSessions(
            response.sessions.filter(
              (session) =>
                session.category === 'needs_action' ||
                session.category === 'later' ||
                session.category === 'waiting' ||
                session.category === 'in_progress' ||
                session.category === 'done',
            ),
          )
          recordSessionChanges(changedSessionIds)
          setSessionsReceivedAt(receivedAt)
          setNowMs(receivedAt)
          setLoaded(true)
        },
        setError,
        { onUpdateAvailable: setAvailableVersion },
      ),
    [recordSessionSnapshot],
  )

  const selectedWaitingIndex = inboxSort.navigableSessions
    .filter((session) => session.category === 'waiting')
    .findIndex((session) => session.id === selectedId)

  useEffect(() => {
    if (isMobile && selectedWaitingIndex >= 0) {
      setWaitingExpanded(true)
    }
  }, [isMobile, selectedId, selectedWaitingIndex, setWaitingExpanded])

  useEffect(() => {
    if (view !== 'inbox') {
      return
    }

    document
      .querySelector<HTMLButtonElement>(
        '[data-session-row][aria-current="true"]',
      )
      ?.scrollIntoView({ block: 'nearest' })
  }, [
    inProgressExpanded,
    selectedId,
    showAllInProgress,
    showAllWaiting,
    view,
    waitingExpanded,
  ])

  useSessionKeyboardNavigation({
    isMobile,
    laterSessionId,
    laterSubmissionInFlight,
    closeLater,
    closeSession: requestCloseSession,
    submitLater,
    navigableSessions: inboxSort.navigableSessions,
    openLater,
    selectedId,
    view,
    navigate,
    toggleInbox,
    toggleStatusPanel,
    focusComposer,
    openNewSession: () => {
      if (newSessionMode === null) openNewSession('form')
    },
  })

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
      navigate((current) => ({
        ...current,
        view: 'inbox',
        conversationOpen: isMobile ? false : current.conversationOpen,
      }))
    },
    [isMobile, navigate, savedViews.selectView],
  )
  const { showConversation: showMobileConversation } =
    useSessionMobileNavigation({
      isMobile,
      conversationOpen: mobileConversationOpen,
      navigableSessions: inboxSort.navigableSessions,
      selectedId,
      selectedSessionExists: selectedSession !== undefined,
      sessionsLoaded: loaded,
      navigate,
    })
  const closeMobileNav = useCallback(() => {
    setMobileNavOpen(false)
  }, [])
  useEffect(() => {
    if (!isMobile || showMobileConversation) closeMobileNav()
  }, [closeMobileNav, isMobile, showMobileConversation])
  const openMobilePalette = useCallback(() => {
    closeMobileNav()
    setCommandPaletteOpen(true)
  }, [closeMobileNav])
  const { closeNewSession, openNewSession, switchFromNewSessionToSession } =
    useNewSessionNavigation({
      backFromNewSession,
      closeMobileNav,
      navigate,
      replaceNewSessionWithSession,
      resetNewSession: newSession.reset,
      setCommandPaletteOpen,
      setPendingLaunchId,
      requestComposerFocus,
    })
  const openSessionFromNavigation = useCallback(
    (id: string) => {
      closeMobileNav()
      openSessionInInbox(id)
    },
    [closeMobileNav, openSessionInInbox],
  )
  usePendingMergeRetryNavigation({
    isMobile,
    view,
    selectedId,
    conversationOpen: mobileConversationOpen,
    newSessionMode,
    sessions: sessionsWithElapsedTime,
    openSessionInInbox: openSessionFromNavigation,
  })
  const selectSavedViewFromNavigation = useCallback(
    (id: string) => {
      closeMobileNav()
      selectInboxSavedView(id)
    },
    [closeMobileNav, selectInboxSavedView],
  )
  useEffect(() => {
    if (pendingLaunchId === null) return
    const launchedSession = findLaunchedSession(
      sessionsWithElapsedTime,
      pendingLaunchId,
    )
    if (launchedSession === undefined) return
    setPendingLaunchId(null)
    const shouldOpenLaunchedSession =
      newSessionMode === 'palette' ||
      (newSessionMode === 'form' && view === 'inbox')
    if (!shouldOpenLaunchedSession) {
      if (newSessionMode === 'form') {
        newSession.reset()
        navigate((current) => ({ ...current, newSessionMode: null }), 'replace')
      }
      return
    }
    setCommandPaletteOpen(false)
    openSessionInInbox(launchedSession.id, 'replace')
  }, [
    navigate,
    newSessionMode,
    newSession.reset,
    openSessionInInbox,
    pendingLaunchId,
    sessionsWithElapsedTime,
    setCommandPaletteOpen,
    view,
  ])
  return (
    <SessionElapsedStartedAtProvider value={sessionsReceivedAt}>
      <AppWorkspace
        state={{
          isMobile,
          view,
          selectedId,
          selectedSession,
          showMobileConversation,
          newSessionMode,
          mobileNavOpen,
          inboxCollapsed,
          sessions: sessionsWithElapsedTime,
          todaySessions,
          sessionRefreshRevisions: sessionRevisions,
          sessionsReceivedAt,
          loaded,
          error,
          inboxFilterQuery,
          inboxSort: inboxSort.sort,
          inboxSortStorageAvailable: inboxSort.storageAvailable,
          showAllInProgress,
          showAllWaiting,
          laterExpanded,
          inProgressExpanded,
          waitingExpanded,
          doneExpanded,
          statusPanelOpen,
          pendingLaunchId,
          projects,
          focusedProjectName,
          hideQuietProjects,
          workTypeNavGroups,
          needsActionCount,
          composerFocusRequest,
          laterSession,
          laterTimes,
          laterTimesError,
          laterMutationPending,
          commandPaletteOpen,
          recentSessionIds,
          workTypeNavLabels,
          pendingFocusSessionId,
          closeConfirmationSession,
        }}
        updateNoticeVersion={
          availableVersion === dismissedVersion ? null : availableVersion
        }
        actions={{
          setInboxFilterQuery,
          setInboxSort: inboxSort.setSort,
          setShowAllInProgress,
          setShowAllWaiting,
          setLaterExpanded,
          setInProgressExpanded,
          setWaitingExpanded,
          setDoneExpanded,
          setMobileNavOpen,
          setStatusPanelOpen,
          setPendingLaunchId,
          setHideQuietProjects,
          setCommandPaletteOpen,
          setFocusedProjectName,
          toggleInbox,
          toggleStatusPanel,
          toggleToday,
          selectInboxSavedView,
          selectSavedViewFromNavigation,
          openSessionFromNavigation,
          openMobilePalette,
          openNewSession,
          closeNewSession,
          switchFromNewSessionToSession,
          openConversation,
          openTodaySession,
          openSessionInInbox,
          closeDoneSessions: (doneSessions) => {
            void closeDoneSessions(doneSessions)
          },
          closeDoneSession,
          requestCloseSession,
          cancelCloseConfirmation,
          confirmCloseSession,
          onLater: openLater,
          closeLater,
          laterSubmissionInFlight,
          submitLater,
          submitBackNow,
          dispatchSessionOperation,
          selectSession,
          orderedSessions,
          onBack: backToInbox,
          onComposerFocusRequestHandled: handleComposerFocusRequest,
          focusSession,
          runPaletteCommand,
          navigate,
          closeMobileNav,
          dismissUpdateNotice: setDismissedVersion,
        }}
        savedViews={savedViews}
        newSession={newSession}
        paletteShortcut={getCommandPaletteShortcut(navigator.platform)}
      />
    </SessionElapsedStartedAtProvider>
  )
}
