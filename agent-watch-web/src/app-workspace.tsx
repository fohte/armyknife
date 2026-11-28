import type { Dispatch, SetStateAction } from 'react'

import type { BoardProject, WorkTypeNavGroup } from '#board-model'
import type { PaletteItem } from '#command-palette-model'
import type { CommandPaletteShortcut } from '#command-palette-shortcut'
import { AgentWatchNav } from '#components/agent-watch-nav'
import { BoardView } from '#components/board-view'
import { CommandPalette } from '#components/command-palette'
import { InboxConversationPanes } from '#components/inbox-conversation-panes'
import { LaterMenu } from '#components/later-menu'
import { MobileNavDrawer } from '#components/mobile-nav-drawer'
import { NewSessionForm } from '#components/new-session-form'
import { NewSessionPicker } from '#components/new-session-picker'
import { ResizableColumns } from '#components/resizable-columns'
import { SessionCloseConfirmation } from '#components/session-close-confirmation'
import { SessionDiscardConfirmation } from '#components/session-discard-confirmation'
import { UpdateNotice } from '#components/update-notice'
import type { InboxSort } from '#inbox-sort'
import type { LaunchSessionRequest } from '#new-session-api'
import type { useSessionHistory } from '#session-history'
import type { LaterPreset, LaterPresetTimes } from '#session-navigation'
import type { SessionOperation } from '#session-operations'
import type { Session } from '#types'
import type { useNewSession } from '#use-new-session'
import type { useSavedInboxViews } from '#use-saved-inbox-views'
import type { SessionDiscardConfirmationState } from '#use-session-close'
import type { PendingLaunchView } from '#use-session-launch'

type AppWorkspaceState = {
  isMobile: boolean
  view: 'inbox' | 'board'
  selectedId: string | null
  selectedSession: Session | undefined
  showMobileConversation: boolean
  newSessionMode: 'form' | 'palette' | null
  mobileNavOpen: boolean
  inboxCollapsed: boolean
  sessions: readonly Session[]
  todaySessions: readonly Session[]
  sessionRefreshRevisions: ReadonlyMap<string, number>
  sessionsReceivedAt: number
  loaded: boolean
  error: string | null
  inboxFilterQuery: string
  inboxSort: InboxSort
  inboxSortStorageAvailable: boolean
  showAllInProgress: boolean
  showAllWaiting: boolean
  laterExpanded: boolean
  inProgressExpanded: boolean
  waitingExpanded: boolean
  doneExpanded: boolean
  statusPanelOpen: boolean
  pendingLaunch: PendingLaunchView | null
  projects: readonly BoardProject[]
  focusedProjectName: string | null
  hideQuietProjects: boolean
  workTypeNavGroups: readonly WorkTypeNavGroup[]
  needsActionCount: number
  composerFocusRequest: { sessionId: string; requestId: number } | null
  laterSession: Session | undefined
  laterTimes: LaterPresetTimes | null
  laterTimesError: boolean
  laterMutationPending: boolean
  commandPaletteOpen: boolean
  recentSessionIds: readonly string[]
  workTypeNavLabels: Readonly<Record<string, string>>
  pendingFocusSessionId: string | null
  closeConfirmationSession: Session | undefined
  discardConfirmation: SessionDiscardConfirmationState | undefined
}

type AppWorkspaceActions = {
  setInboxFilterQuery: Dispatch<SetStateAction<string>>
  setInboxSort: (sort: InboxSort) => void
  setShowAllInProgress: Dispatch<SetStateAction<boolean>>
  setShowAllWaiting: Dispatch<SetStateAction<boolean>>
  setLaterExpanded: Dispatch<SetStateAction<boolean>>
  setInProgressExpanded: Dispatch<SetStateAction<boolean>>
  setWaitingExpanded: Dispatch<SetStateAction<boolean>>
  setDoneExpanded: Dispatch<SetStateAction<boolean>>
  setMobileNavOpen: Dispatch<SetStateAction<boolean>>
  setStatusPanelOpen: Dispatch<SetStateAction<boolean>>
  onStart: (request: LaunchSessionRequest) => void
  setHideQuietProjects: Dispatch<SetStateAction<boolean>>
  setCommandPaletteOpen: Dispatch<SetStateAction<boolean>>
  setFocusedProjectName: Dispatch<SetStateAction<string | null>>
  toggleInbox: () => void
  toggleStatusPanel: () => void
  toggleToday: (id: string, isToday: boolean) => void
  selectInboxSavedView: (id: string) => void
  selectSavedViewFromNavigation: (id: string) => void
  openSessionFromNavigation: (id: string) => void
  openMobilePalette: () => void
  openNewSession: (mode: 'form' | 'palette') => void
  closeNewSession: () => void
  switchFromNewSessionToSession: (id: string, openConversation: boolean) => void
  openConversation: (id: string) => void
  openTodaySession: (id: string) => void
  openSessionInInbox: (id: string) => void
  closeDoneSessions: (sessions: readonly Session[]) => void
  closeDoneSession: (id: string, keepWorktree?: boolean) => void
  requestCloseSession: (id: string) => void
  cancelCloseConfirmation: () => void
  confirmCloseSession: (id: string) => void
  cancelDiscardConfirmation: () => void
  confirmDiscardChanges: () => void
  onLater: (id: string) => void
  closeLater: () => void
  laterSubmissionInFlight: { current: boolean }
  submitLater: (id: string, preset: LaterPreset) => void
  submitBackNow: (id: string) => void
  dispatchSessionOperation: (operation: SessionOperation) => void
  selectSession: (id: string) => void
  orderedSessions: readonly Session[]
  onBack: () => void
  onComposerFocusRequestHandled: (requestId: number) => void
  focusSession: (id: string) => unknown
  runPaletteCommand: (
    command: Extract<PaletteItem, { type: 'command' }>,
  ) => unknown
  navigate: ReturnType<typeof useSessionHistory>['navigate']
  closeMobileNav: () => void
  dismissUpdateNotice: (version: string) => void
}

type AppWorkspaceProps = {
  state: AppWorkspaceState
  actions: AppWorkspaceActions
  updateNoticeVersion: string | null
  savedViews: ReturnType<typeof useSavedInboxViews>
  newSession: ReturnType<typeof useNewSession>
  paletteShortcut: CommandPaletteShortcut
}

export function AppWorkspace({
  state,
  actions,
  updateNoticeVersion,
  savedViews,
  newSession,
  paletteShortcut,
}: AppWorkspaceProps) {
  const laterSession = state.laterSession
  const closeConfirmationSession = state.closeConfirmationSession
  const mobileUpdateNotice =
    state.isMobile && updateNoticeVersion !== null ? (
      <UpdateNotice
        isMobile
        onDismiss={() => {
          actions.dismissUpdateNotice(updateNoticeVersion)
        }}
        onReload={() => {
          window.location.reload()
        }}
        version={updateNoticeVersion}
      />
    ) : undefined
  const inboxProps = {
    error: state.error ?? savedViews.storageError,
    connectionError: state.error,
    lastUpdatedAt: state.sessionsReceivedAt,
    loaded: state.loaded,
    sort: state.inboxSort,
    sortStorageAvailable: state.inboxSortStorageAvailable,
    onSortChange: actions.setInboxSort,
    selectedId: state.selectedId,
    sessions: state.sessions,
    filterQuery: state.inboxFilterQuery,
    onFilterQueryChange: actions.setInboxFilterQuery,
    saveViewOpen: savedViews.saveViewOpen,
    saveViewName: savedViews.saveViewName,
    onSaveViewOpenChange: savedViews.setSaveViewOpen,
    onSaveViewNameChange: savedViews.setSaveViewName,
    onSaveView: savedViews.saveView,
    savedViews: savedViews.savedViewsForNav,
    selectedSavedViewId: savedViews.activeSavedViewId,
    onSelectSavedView: actions.selectInboxSavedView,
    onDeleteSavedView: savedViews.deleteView,
    showAllInProgress: state.showAllInProgress,
    onShowMore: () => {
      actions.setShowAllInProgress(true)
    },
    showAllWaiting: state.showAllWaiting,
    onShowMoreWaiting: () => {
      actions.setShowAllWaiting(true)
    },
    onSelect: (id: string) => {
      if (state.newSessionMode === 'form') {
        actions.switchFromNewSessionToSession(id, true)
        return
      }
      actions.openConversation(id)
    },
    onLater: actions.onLater,
    onCloseSession: actions.requestCloseSession,
    laterExpanded: state.laterExpanded,
    onToggleLater: () => {
      actions.setLaterExpanded((expanded) => !expanded)
    },
    onToggleToday: actions.toggleToday,
    todaySessions: state.todaySessions,
    onSelectToday: actions.openTodaySession,
    inProgressExpanded: state.inProgressExpanded,
    onToggleInProgress: () => {
      actions.setInProgressExpanded((expanded) => !expanded)
    },
    waitingExpanded: state.waitingExpanded,
    onToggleWaiting: () => {
      actions.setWaitingExpanded((expanded) => !expanded)
    },
    doneExpanded: state.doneExpanded,
    onToggleDone: () => {
      actions.setDoneExpanded((expanded) => !expanded)
    },
    onCloseDone: actions.closeDoneSessions,
    onOpenNavigation: () => {
      actions.setMobileNavOpen(true)
    },
    onOpenPalette: actions.openMobilePalette,
    onNewSession: () => {
      actions.openNewSession('form')
    },
  }
  const navigation = (
    <AgentWatchNav
      activeView={state.view}
      isMobile={state.isMobile}
      paletteShortcut={paletteShortcut}
      needsActionCount={state.needsActionCount}
      savedViews={savedViews.savedViewsForNav}
      selectedSavedViewId={savedViews.activeSavedViewId}
      todaySessions={state.todaySessions}
      sessions={state.sessions}
      workTypeNavGroups={state.workTypeNavGroups}
      onViewChange={(nextView) => {
        actions.navigate((current) => ({
          ...current,
          view: nextView,
          conversationOpen: state.isMobile ? false : current.conversationOpen,
        }))
        actions.closeMobileNav()
      }}
      onSelectSavedView={actions.selectSavedViewFromNavigation}
      onDeleteSavedView={savedViews.deleteView}
      onSelectToday={actions.openSessionFromNavigation}
      onSelectSession={actions.openSessionFromNavigation}
      onOpenPalette={actions.openMobilePalette}
      onNewSession={() => {
        actions.openNewSession('form')
      }}
      inboxCollapsed={state.inboxCollapsed}
      onToggleInbox={actions.toggleInbox}
      onCloseNavigation={actions.closeMobileNav}
    />
  )
  const main = (
    <main className="agent-watch-main">
      {state.view === 'inbox' ? (
        <InboxConversationPanes
          inboxProps={inboxProps}
          sessions={state.sessions}
          selectedSession={state.selectedSession}
          sessionRefreshRevisions={state.sessionRefreshRevisions}
          isMobile={state.isMobile}
          showMobileConversation={state.showMobileConversation}
          inboxCollapsed={state.inboxCollapsed}
          statusPanelOpen={state.statusPanelOpen}
          mobileNavOpen={state.mobileNavOpen}
          onOpenNavigation={() => {
            actions.setMobileNavOpen(true)
          }}
          onToggleStatusPanel={actions.toggleStatusPanel}
          onCloseStatusPanel={() => {
            actions.setStatusPanelOpen(false)
          }}
          onBack={actions.onBack}
          newSessionPane={
            state.newSessionMode === 'form' ? (
              <NewSessionForm
                isMobile={state.isMobile}
                pendingLaunch={state.pendingLaunch}
                mobileUpdateNotice={mobileUpdateNotice}
                onClose={actions.closeNewSession}
                onStart={actions.onStart}
                state={newSession}
              />
            ) : undefined
          }
          showMobileNewSession={state.newSessionMode === 'form'}
          onLater={actions.onLater}
          onCloseSession={actions.requestCloseSession}
          onOptimisticSessionOperation={actions.dispatchSessionOperation}
          onSelectSession={(id) => {
            if (!actions.orderedSessions.some((session) => session.id === id)) {
              return
            }
            if (state.newSessionMode === 'form') {
              actions.switchFromNewSessionToSession(id, false)
              return
            }
            actions.selectSession(id)
          }}
          composerFocusRequest={state.composerFocusRequest}
          onComposerFocusRequestHandled={actions.onComposerFocusRequestHandled}
          mobileUpdateNotice={mobileUpdateNotice}
        />
      ) : (
        <BoardView
          error={state.error}
          hideQuietProjects={state.hideQuietProjects}
          loaded={state.loaded}
          onHideQuietProjectsChange={actions.setHideQuietProjects}
          onNewSession={() => {
            actions.openNewSession('form')
          }}
          onSelectSession={actions.openSessionInInbox}
          projects={state.projects}
          focusedProjectName={state.focusedProjectName}
          isMobile={state.isMobile}
          mobileUpdateNotice={mobileUpdateNotice}
          onOpenNavigation={() => {
            actions.setMobileNavOpen(true)
          }}
          onOpenPalette={actions.openMobilePalette}
        />
      )}
    </main>
  )

  return (
    <div
      className="agent-watch-app"
      data-layout={state.isMobile ? 'mobile' : 'desktop'}
    >
      {!state.isMobile && updateNoticeVersion !== null ? (
        <UpdateNotice
          isMobile={false}
          onDismiss={() => {
            actions.dismissUpdateNotice(updateNoticeVersion)
          }}
          onReload={() => {
            window.location.reload()
          }}
          version={updateNoticeVersion}
        />
      ) : null}
      <div
        className="agent-watch-shell"
        data-layout={state.isMobile ? 'mobile' : 'desktop'}
        data-mobile-screen={
          state.showMobileConversation || state.newSessionMode === 'form'
            ? 'conversation'
            : 'inbox'
        }
      >
        {state.isMobile ? (
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
        {state.isMobile ? (
          <MobileNavDrawer
            onClose={actions.closeMobileNav}
            open={state.mobileNavOpen}
          >
            {state.showMobileConversation && !state.mobileNavOpen
              ? null
              : navigation}
          </MobileNavDrawer>
        ) : null}
        {laterSession !== undefined ? (
          <LaterMenu
            sessionTitle={laterSession.title}
            isLater={laterSession.category === 'later'}
            laterTimes={state.laterTimes}
            laterTimesError={state.laterTimesError}
            isSubmitting={state.laterMutationPending}
            isMobile={state.isMobile}
            onClose={() => {
              if (!actions.laterSubmissionInFlight.current) {
                actions.closeLater()
              }
            }}
            onBackNow={() => {
              actions.submitBackNow(laterSession.id)
            }}
            onSelect={(preset) => {
              actions.submitLater(laterSession.id, preset)
            }}
          />
        ) : null}
        {state.commandPaletteOpen ? (
          <CommandPalette
            onClose={() => {
              actions.setCommandPaletteOpen(false)
            }}
            onLater={(id) => {
              actions.setCommandPaletteOpen(false)
              actions.onLater(id)
            }}
            onCloseSession={(id) => {
              actions.setCommandPaletteOpen(false)
              actions.requestCloseSession(id)
            }}
            onOpenInTerminal={(id) => {
              void actions.focusSession(id)
            }}
            onOpenProject={(project) => {
              actions.setCommandPaletteOpen(false)
              actions.setFocusedProjectName(project)
              actions.navigate((current) => ({
                ...current,
                selectedId: null,
                view: 'board',
                conversationOpen: false,
              }))
            }}
            onOpenSession={(id) => {
              actions.setCommandPaletteOpen(false)
              actions.openSessionInInbox(id)
            }}
            onOpenSessionBeside={(id) => {
              const query = new URLSearchParams({ session: id })
              if (state.isMobile) query.set('screen', 'conversation')
              window.open(
                `?${query.toString()}`,
                '_blank',
                'noopener,noreferrer',
              )
              actions.setCommandPaletteOpen(false)
            }}
            onRunCommand={(command) => {
              if (command.id === 'new-session') {
                actions.openNewSession('palette')
                return
              }
              void actions.runPaletteCommand(command)
            }}
            recentSessionIds={state.recentSessionIds}
            sessions={state.sessions}
            workTypeNavLabels={state.workTypeNavLabels}
            pendingFocusSessionId={state.pendingFocusSessionId}
          />
        ) : null}
        {state.newSessionMode === 'palette' ? (
          <NewSessionPicker
            isMobile={state.isMobile}
            launchPending={state.pendingLaunch !== null}
            onClose={actions.closeNewSession}
            onStart={actions.onStart}
            state={newSession}
          />
        ) : null}
        {closeConfirmationSession === undefined ? null : (
          <SessionCloseConfirmation
            session={closeConfirmationSession}
            mobile={state.isMobile}
            onCancel={actions.cancelCloseConfirmation}
            onConfirm={() => {
              actions.confirmCloseSession(closeConfirmationSession.id)
            }}
          />
        )}
        {state.discardConfirmation === undefined ? null : (
          <SessionDiscardConfirmation
            sessionName={state.discardConfirmation.sessionName}
            branchName={state.discardConfirmation.branchName}
            worktreePath={state.discardConfirmation.worktreePath}
            changes={state.discardConfirmation.changes}
            commits={state.discardConfirmation.commits}
            reviewAgain={state.discardConfirmation.reviewAgain}
            mobile={state.isMobile}
            onCancel={actions.cancelDiscardConfirmation}
            onDiscard={actions.confirmDiscardChanges}
          />
        )}
      </div>
    </div>
  )
}
