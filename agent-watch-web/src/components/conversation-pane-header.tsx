import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import {
  ChevronLeft,
  Menu as MenuIcon,
  PanelRightClose,
  PanelRightOpen,
  Pencil,
  Star,
} from 'lucide-react'
import { type SubmitEvent, useState } from 'react'
import { createPortal } from 'react-dom'

import { DesktopConversationActionsMenu } from '#components/desktop-conversation-actions-menu'
import { MergeBand } from '#components/merge-band'
import { MobileConversationActionsMenu } from '#components/mobile-conversation-actions-menu'
import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { PullRequestMergeConfirmation } from '#components/pull-request-merge-confirmation'
import { SessionHeading } from '#components/session-heading'
import {
  pullRequestMergeReason,
  SessionPullRequests,
  SessionPullRequestStatusBadge,
  sessionPullRequestStatusLabel,
} from '#components/session-pull-requests'
import { SessionTaskLinks } from '#components/session-task-links'
import { StarredSessionToggle } from '#components/starred-session-toggle'
import { useConversationPaneHeaderMerge } from '#components/use-conversation-pane-header-merge'
import { VibeIndicator } from '#components/vibe-indicator'
import { VibeModeControl } from '#components/vibe-mode-control'
import { getSessionTasks } from '#session-task-utils'
import type {
  BackgroundTask,
  OpenInTerminalStatus,
  Session,
  SessionActions,
  VibeMode,
} from '#types'

type ConversationPaneHeaderProps = {
  session: Session
  timelineIndex?: number
  vibeMode?: VibeMode | undefined
  actions: SessionActions | null
  isMobile: boolean
  statusPanelOpen: boolean
  backgroundTasks: readonly BackgroundTask[]
  openInTerminalStatus: OpenInTerminalStatus
  isToday: boolean
  titleEditing?: boolean
  mobileActionsMenuOpen?: boolean
  pullRequestsOpen?: boolean
  pullRequestListTarget?: HTMLElement | null
  activityFailed?: boolean
  mobileNavOpen?: boolean
  mergeBandTarget?: HTMLElement | null
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onOpenInTerminal: () => void
  onSelectSession: (id: string) => void
  onToggleStatusPanel: () => void
  onOpenTasks?: () => void
  onToggleToday?: (isToday: boolean) => void
  onBack?: () => void
  onOpenNavigation?: () => void
  onLater?: () => void
  onClose?: () => void
}

export function ConversationPaneHeader({
  session,
  timelineIndex,
  vibeMode = session.vibeMode,
  actions,
  isMobile,
  statusPanelOpen,
  backgroundTasks,
  openInTerminalStatus,
  isToday,
  titleEditing = false,
  mobileActionsMenuOpen,
  pullRequestsOpen,
  pullRequestListTarget = null,
  activityFailed = false,
  mobileNavOpen = false,
  mergeBandTarget = null,
  onAction,
  onOpenInTerminal,
  onSelectSession,
  onToggleStatusPanel,
  onOpenTasks = () => undefined,
  onToggleToday,
  onBack,
  onOpenNavigation,
  onLater,
  onClose,
}: ConversationPaneHeaderProps) {
  const [isEditingTitle, setIsEditingTitle] = useState(titleEditing)
  const [titleDraft, setTitleDraft] = useState(session.title)
  const [isSavingTitle, setIsSavingTitle] = useState(false)
  const [internalPullRequestListOpen, setInternalPullRequestListOpen] =
    useState(false)
  const pullRequestListOpen = pullRequestsOpen ?? internalPullRequestListOpen
  const setPullRequestListOpen = (open: boolean) => {
    setInternalPullRequestListOpen(open)
  }
  const [mobileVibeOpen, setMobileVibeOpen] = useState(false)
  const parent = session.parent
  const pullRequests =
    session.pullRequests !== undefined && session.pullRequests.length > 0
      ? session.pullRequests
      : session.pullRequest === null
        ? []
        : [session.pullRequest]
  const merge = useConversationPaneHeaderMerge({
    session,
    pullRequests,
    ...(timelineIndex === undefined ? {} : { timelineIndex }),
    setPullRequestListOpen,
  })
  const mergeRows = merge.mergeablePullRequests.flatMap((target) => {
    const status = merge.mergeStatusFor(target)
    if (status === 'merged' || status === 'alreadyMerged') return []
    const canRetry = status === 'mergeFailed' && merge.isMergeable(target)
    const error =
      status === 'mergeFailed'
        ? canRetry
          ? merge.mergeActionFor(target)?.error
          : pullRequestMergeReason(target)
        : undefined
    return [
      {
        pullRequest: target,
        status:
          status === null
            ? ('ready' as const)
            : status === 'mergeFailed'
              ? ('mergeFailed' as const)
              : status,
        ...(status === 'mergeFailed' ? { canRetry } : {}),
        ...(error === undefined ? {} : { error }),
      },
    ]
  })
  const hasMergeFailure = mergeRows.some(
    ({ status }) => status === 'mergeFailed',
  )
  const mergeStatusFor = (target: (typeof pullRequests)[number]) =>
    merge.mergeStatusFor(target)
  const pullRequestStatusLabel = sessionPullRequestStatusLabel(
    pullRequests,
    mergeStatusFor,
  )
  const mobileStatusPanelLabel = `${statusPanelOpen ? 'Close' : 'Open'} session status panel${pullRequestStatusLabel === null ? '' : `. ${pullRequestStatusLabel}`}`
  const mergeErrorFor = (target: (typeof pullRequests)[number]) =>
    merge.mergeActionFor(target)?.error
  const canRetryMerge = (target: (typeof pullRequests)[number]) =>
    merge.isMergeable(target)
  const retryMerge = (target: (typeof pullRequests)[number]) => {
    const action = merge.mergeActionFor(target)
    if (action?.status === 'mergeFailed') merge.retryMerge(action.id)
  }
  const mergeBand =
    mergeBandTarget === null
      ? null
      : createPortal(
          <MergeBand
            rows={mergeRows}
            isMobile={isMobile}
            onMerge={merge.openMergeConfirmation}
            onRetry={retryMerge}
          />,
          mergeBandTarget,
        )
  const linkedTasks = getSessionTasks(session)
  const mobileRepoName = session.repo.split('/').at(-1) ?? session.repo

  const saveTitle = async (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    setIsSavingTitle(true)
    const saved = await onAction('title', { title: titleDraft })
    setIsSavingTitle(false)
    if (saved) setIsEditingTitle(false)
  }

  const editTitle = () => {
    setTitleDraft(session.title)
    setIsEditingTitle(true)
  }

  const handleTitleSubmit = (event: SubmitEvent<HTMLFormElement>) => {
    void saveTitle(event)
  }

  const titleHeading = isEditingTitle ? (
    <form className="session-title-editor" onSubmit={handleTitleSubmit}>
      <input
        aria-label="Session title"
        autoFocus
        value={titleDraft}
        disabled={isSavingTitle}
        onChange={(event) => {
          setTitleDraft(event.currentTarget.value)
        }}
      />
      <Button
        className="cursor-pointer"
        type="submit"
        variant="plain"
        disabled={isSavingTitle}
      >
        {isSavingTitle ? 'Saving…' : 'Save'}
      </Button>
      <Button
        className="cursor-pointer"
        type="button"
        variant="plain"
        disabled={isSavingTitle}
        onClick={() => {
          setTitleDraft(session.title)
          setIsEditingTitle(false)
        }}
      >
        Cancel
      </Button>
    </form>
  ) : (
    <div className="session-title-heading">
      <SessionHeading
        title={session.title}
        {...(isMobile ? {} : { sessionId: session.id })}
      />
      {isMobile || vibeMode === undefined ? null : (
        <VibeModeControl
          sessionId={session.id}
          repo={session.repo}
          mode={vibeMode}
        />
      )}
      {isMobile ? null : (
        <Button
          className="session-title-edit cursor-pointer"
          type="button"
          variant="plain"
          aria-label="Edit session title"
          title="Edit session title"
          onClick={editTitle}
        >
          <Pencil aria-hidden="true" size={15} />
        </Button>
      )}
    </div>
  )

  return (
    <header className="conversation-header">
      {isMobile ? (
        <>
          <div className="mobile-conversation-navigation">
            <Button
              className="mobile-conversation-back cursor-pointer"
              type="button"
              variant="plain"
              aria-label="Back to Inbox"
              onClick={onBack}
            >
              <ChevronLeft aria-hidden="true" size={30} strokeWidth={2.5} />
            </Button>
            <Button
              className="mobile-conversation-nav-toggle cursor-pointer"
              type="button"
              variant="plain"
              aria-label={
                activityFailed
                  ? 'Open navigation, Activity has failed actions'
                  : 'Open navigation'
              }
              aria-haspopup="dialog"
              aria-expanded={mobileNavOpen}
              onClick={onOpenNavigation}
            >
              <MenuIcon aria-hidden="true" size={22} />
              {activityFailed ? (
                <span
                  className="mobile-conversation-nav-failure"
                  aria-hidden="true"
                />
              ) : null}
            </Button>
          </div>
          <div className="mobile-conversation-title">
            {isToday ? (
              <Star
                aria-label="Starred"
                className="mobile-conversation-starred"
                role="img"
                size={13}
              />
            ) : null}
            {titleHeading}
            {vibeMode === undefined ? null : (
              <VibeIndicator mode={vibeMode} size={13} />
            )}
          </div>
          <div className="mobile-conversation-header-actions">
            <Button
              className={`mobile-conversation-status-toggle${hasMergeFailure ? ' is-merge-failed' : ''}`}
              type="button"
              variant="plain"
              aria-label={mobileStatusPanelLabel}
              aria-expanded={statusPanelOpen}
              aria-controls="session-status-panel"
              title={mobileStatusPanelLabel}
              onClick={() => {
                onToggleStatusPanel()
              }}
            >
              {statusPanelOpen ? (
                <PanelRightClose aria-hidden="true" size={18} />
              ) : (
                <PanelRightOpen aria-hidden="true" size={18} />
              )}
              {pullRequests.length > 0 ? (
                <SessionPullRequestStatusBadge
                  pullRequests={pullRequests}
                  mergeStatus={mergeStatusFor}
                />
              ) : null}
            </Button>
            <MobileConversationActionsMenu
              sessionId={session.id}
              vibeMode={vibeMode}
              isDone={session.category === 'done'}
              isStarred={isToday}
              {...(mobileActionsMenuOpen === undefined
                ? {}
                : { open: mobileActionsMenuOpen })}
              {...(onLater === undefined ? {} : { onLater })}
              {...(onClose === undefined ? {} : { onClose })}
              {...(onToggleToday === undefined
                ? {}
                : { onToggleStarred: onToggleToday })}
              onEditTitle={editTitle}
              onOpenVibe={() => {
                setMobileVibeOpen(true)
              }}
            />
            {vibeMode === undefined ? null : (
              <VibeModeControl
                sessionId={session.id}
                repo={session.repo}
                mode={vibeMode}
                isMobile
                open={mobileVibeOpen}
                onOpenChange={setMobileVibeOpen}
              />
            )}
          </div>
          <div className="mobile-conversation-context">
            <SessionTaskLinks
              task={session.task}
              tasks={linkedTasks}
              onOpenTasks={onOpenTasks}
            />
            {session.task ? <span aria-hidden="true">·</span> : null}
            <span className="mobile-conversation-repo">{mobileRepoName}</span>
          </div>
        </>
      ) : (
        <>
          <div className="conversation-header-copy">
            <div className="conversation-context-line">
              <SessionTaskLinks
                task={session.task}
                tasks={linkedTasks}
                onOpenTasks={onOpenTasks}
              />
              <a
                className="conversation-repo"
                href={`https://github.com/${session.repo}`}
                target="_blank"
                rel="noreferrer"
              >
                {session.repo}
              </a>
              {parent != null ? (
                <a
                  className="conversation-parent"
                  href={`#${encodeURIComponent(parent.id)}`}
                  onClick={(event) => {
                    event.preventDefault()
                    onSelectSession(parent.id)
                  }}
                >
                  ↑ Parent: {parent.title ?? 'session'}
                </a>
              ) : null}
              <div className="conversation-header-actions">
                <Button
                  className="conversation-status-panel-toggle"
                  type="button"
                  variant="plain"
                  aria-label={
                    statusPanelOpen
                      ? 'Close session status panel'
                      : 'Open session status panel'
                  }
                  aria-expanded={statusPanelOpen}
                  aria-controls="session-status-panel"
                  title={
                    statusPanelOpen
                      ? 'Close session status panel'
                      : 'Open session status panel'
                  }
                  onClick={() => {
                    onToggleStatusPanel()
                  }}
                >
                  {statusPanelOpen ? (
                    <PanelRightClose aria-hidden="true" size={15} />
                  ) : (
                    <PanelRightOpen aria-hidden="true" size={15} />
                  )}
                  {backgroundTasks.length > 0 ? (
                    <span className="conversation-status-panel-count">
                      {backgroundTasks.length}
                    </span>
                  ) : null}
                </Button>
                <StarredSessionToggle
                  className="conversation-today-toggle"
                  size="desktop"
                  isStarred={isToday}
                  onToggleToday={onToggleToday}
                />
                <SessionPullRequests
                  pullRequests={pullRequests}
                  repo={session.repo}
                  isMobile={false}
                  open={pullRequestListOpen}
                  mergeStatus={mergeStatusFor}
                  canRetryMerge={canRetryMerge}
                  mergeError={mergeErrorFor}
                  onMerge={merge.openMergeConfirmation}
                  onRetryMerge={retryMerge}
                  onOpenChange={setPullRequestListOpen}
                />
                {session.category === 'done' && onClose !== undefined ? (
                  <Button
                    className="conversation-close cursor-pointer"
                    type="button"
                    variant="plain"
                    onClick={onClose}
                  >
                    Close
                  </Button>
                ) : session.category !== 'done' && onClose !== undefined ? (
                  <DesktopConversationActionsMenu onClose={onClose} />
                ) : null}
                {actions?.openInTerminal === true ? (
                  <OpenInTerminalAction
                    status={openInTerminalStatus}
                    onClick={onOpenInTerminal}
                  />
                ) : null}
              </div>
            </div>
            {titleHeading}
          </div>
        </>
      )}
      {isMobile && pullRequestListTarget !== null && pullRequests.length > 0
        ? createPortal(
            <SessionPullRequests
              pullRequests={pullRequests}
              repo={session.repo}
              isMobile
              presentation="list"
              open={false}
              mergeStatus={mergeStatusFor}
              canRetryMerge={canRetryMerge}
              mergeError={mergeErrorFor}
              onMerge={merge.openMergeConfirmation}
              onRetryMerge={retryMerge}
              onOpenChange={setPullRequestListOpen}
            />,
            pullRequestListTarget,
          )
        : null}
      {mergeBand}
      {merge.mergeConfirmationOpen && merge.mergeTarget !== null ? (
        <PullRequestMergeConfirmation
          number={merge.mergeTarget.number}
          repository={`${merge.mergeTarget.owner}/${merge.mergeTarget.repo}`}
          headSha={merge.mergeTarget.expectedHeadSha}
          merging={merge.isMerging(merge.mergeTarget)}
          mobile={isMobile}
          onCancel={merge.closeMergeConfirmation}
          onConfirm={() => {
            merge.confirmMerge()
          }}
        />
      ) : null}
    </header>
  )
}
