import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import {
  ChevronLeft,
  PanelRightClose,
  PanelRightOpen,
  Pencil,
  Star,
} from 'lucide-react'
import { type SubmitEvent, useState } from 'react'

import { DesktopConversationActionsMenu } from '#components/desktop-conversation-actions-menu'
import { MobileConversationActionsMenu } from '#components/mobile-conversation-actions-menu'
import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { PullRequestMergeAction } from '#components/pull-request-merge-action'
import { PullRequestMergeConfirmation } from '#components/pull-request-merge-confirmation'
import { SessionHeading } from '#components/session-heading'
import {
  getRepresentativePullRequest,
  SessionPullRequests,
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
  onLater?: () => void
  onClose?: () => void
}

export function ConversationPaneHeader({
  session,
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
  onAction,
  onOpenInTerminal,
  onSelectSession,
  onToggleStatusPanel,
  onOpenTasks = () => undefined,
  onToggleToday,
  onBack,
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
  const pullRequest = getRepresentativePullRequest(pullRequests) ?? null
  const merge = useConversationPaneHeaderMerge({
    session,
    pullRequests,
    setPullRequestListOpen,
  })
  const mergeAction =
    !merge.showHeaderMergeAction ||
    merge.headerMergeOperation === null ? null : (
      <PullRequestMergeAction
        merging={merge.isMerging(merge.headerMergeOperation)}
        result={
          merge.mergeActionFor(merge.headerMergeOperation)?.status === 'merged'
            ? 'merged'
            : merge.mergeActionFor(merge.headerMergeOperation)?.status ===
                'alreadyMerged'
              ? 'alreadyMerged'
              : merge.mergeActionFor(merge.headerMergeOperation)?.status ===
                  'queued'
                ? 'queued'
                : merge.mergeActionFor(merge.headerMergeOperation)?.status ===
                    'mergeFailed'
                  ? 'failed'
                  : null
        }
        onClick={() => {
          const target = merge.headerMergeOperation
          if (pullRequest === null || target === null) return
          const action = merge.mergeActionFor(target)
          if (action?.status === 'mergeFailed') {
            merge.retryMerge(action.id)
            return
          }
          merge.openMergeConfirmation(pullRequest)
        }}
      />
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
          <Button
            className="mobile-conversation-back cursor-pointer"
            type="button"
            variant="plain"
            aria-label="Back to Inbox"
            onClick={onBack}
          >
            <ChevronLeft aria-hidden="true" size={30} strokeWidth={2.5} />
          </Button>
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
              className="mobile-conversation-status-toggle"
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
                <PanelRightClose aria-hidden="true" size={18} />
              ) : (
                <PanelRightOpen aria-hidden="true" size={18} />
              )}
            </Button>
            <SessionPullRequests
              pullRequests={pullRequests}
              repo={session.repo}
              isMobile
              open={pullRequestListOpen}
              onMerge={merge.openMergeConfirmation}
              isMerging={merge.isMerging}
              isMergeSuccessful={merge.isMergeSuccessful}
              mergeStatus={(target) =>
                merge.mergeActionFor(target)?.status ?? null
              }
              onRetryMerge={(target) => {
                const action = merge.mergeActionFor(target)
                if (action?.status === 'mergeFailed')
                  merge.retryMerge(action.id)
              }}
              onOpenChange={setPullRequestListOpen}
            />
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
                  onMerge={merge.openMergeConfirmation}
                  isMerging={merge.isMerging}
                  isMergeSuccessful={merge.isMergeSuccessful}
                  mergeStatus={(target) =>
                    merge.mergeActionFor(target)?.status ?? null
                  }
                  onRetryMerge={(target) => {
                    const action = merge.mergeActionFor(target)
                    if (action?.status === 'mergeFailed')
                      merge.retryMerge(action.id)
                  }}
                  onOpenChange={setPullRequestListOpen}
                />
                {mergeAction}
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
      {merge.mergeConfirmationOpen && merge.mergeTarget !== null ? (
        <PullRequestMergeConfirmation
          number={merge.mergeTarget.number}
          headSha={merge.mergeTarget.expectedHeadSha}
          merging={merge.isMerging(merge.mergeTarget)}
          onCancel={merge.closeMergeConfirmation}
          onConfirm={() => {
            merge.confirmMerge()
          }}
        />
      ) : null}
    </header>
  )
}
