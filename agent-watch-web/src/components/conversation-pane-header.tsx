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

import { MobileConversationActionsMenu } from '#components/mobile-conversation-actions-menu'
import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { PullRequestMergeAction } from '#components/pull-request-merge-action'
import { PullRequestMergeConfirmation } from '#components/pull-request-merge-confirmation'
import {
  PullRequestMergeFeedback,
  type PullRequestMergeFeedbackKind,
} from '#components/pull-request-merge-feedback'
import { SessionHeading } from '#components/session-heading'
import {
  getRepresentativePullRequest,
  isPullRequestMergeable,
  SessionPullRequests,
} from '#components/session-pull-requests'
import { SessionTaskLinks } from '#components/session-task-links'
import { StarredSessionToggle } from '#components/starred-session-toggle'
import { VibeIndicator } from '#components/vibe-indicator'
import { VibeModeControl } from '#components/vibe-mode-control'
import { useSessionOperations } from '#session-operation-context'
import type { SessionOperation } from '#session-operations'
import { getSessionTasks } from '#session-task-utils'
import type {
  BackgroundTask,
  OpenInTerminalStatus,
  PullRequest,
  Session,
  SessionActions,
  VibeMode,
} from '#types'

type MergeResponse =
  | { result: 'merged' }
  | { result: 'alreadyMerged' }
  | { result: 'enqueued'; message: string }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

type MergeFeedback = {
  number: number
  owner: string
  repo: string
  kind: PullRequestMergeFeedbackKind
  message: string
}

type PullRequestMergeOperation = Extract<SessionOperation, { type: 'merge' }>

function isMergeResponse(value: unknown): value is MergeResponse {
  if (!isRecord(value)) return false
  const result = value['result']
  return (
    result === 'merged' ||
    result === 'alreadyMerged' ||
    (result === 'enqueued' && typeof value['message'] === 'string')
  )
}

type ConversationPaneHeaderProps = {
  session: Session
  vibeMode?: VibeMode | undefined
  sessions: readonly Session[]
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
  onToggleToday?: (isToday: boolean) => void
  onBack?: () => void
  onLater?: () => void
  onClose?: () => void
}

export function ConversationPaneHeader({
  session,
  vibeMode = session.vibeMode,
  sessions,
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
  onToggleToday,
  onBack,
  onLater,
  onClose,
}: ConversationPaneHeaderProps) {
  const [isEditingTitle, setIsEditingTitle] = useState(titleEditing)
  const [titleDraft, setTitleDraft] = useState(session.title)
  const [isSavingTitle, setIsSavingTitle] = useState(false)
  const [mergeConfirmationOpen, setMergeConfirmationOpen] = useState(false)
  const [mergeTarget, setMergeTarget] =
    useState<PullRequestMergeOperation | null>(null)
  const [internalPullRequestListOpen, setInternalPullRequestListOpen] =
    useState(false)
  const pullRequestListOpen = pullRequestsOpen ?? internalPullRequestListOpen
  const setPullRequestListOpen = (open: boolean) => {
    setInternalPullRequestListOpen(open)
  }
  const [mergeFeedback, setMergeFeedback] = useState<MergeFeedback | null>(null)
  const [mobileVibeOpen, setMobileVibeOpen] = useState(false)
  const { pendingOperations, run } = useSessionOperations()
  const parent = session.parent
  const pullRequests =
    session.pullRequests !== undefined && session.pullRequests.length > 0
      ? session.pullRequests
      : session.pullRequest === null
        ? []
        : [session.pullRequest]
  const pullRequest = getRepresentativePullRequest(pullRequests) ?? null
  const sessionRepoParts = session.repo.split('/')
  const mergeIdentity = (target: {
    number: number
    owner?: string
    repo?: string
  }) => ({
    number: target.number,
    owner: target.owner ?? sessionRepoParts[0],
    repo: target.repo ?? sessionRepoParts.at(-1),
  })
  const mergeOperationFor = (
    target: PullRequest,
  ): PullRequestMergeOperation | null => {
    if (!isPullRequestMergeable(target)) return null
    const identity = mergeIdentity(target)
    if (identity.owner === undefined || identity.repo === undefined) return null
    return {
      type: 'merge',
      sessionId: session.id,
      number: identity.number,
      owner: identity.owner,
      repo: identity.repo,
      expectedHeadSha: target.headSha,
    }
  }
  const headerMergeOperation =
    pullRequest?.isCurrentBranch === true
      ? mergeOperationFor(pullRequest)
      : null
  const isMerging = (target: {
    number: number
    owner?: string
    repo?: string
  }) => {
    const identity = mergeIdentity(target)
    if (identity.owner === undefined || identity.repo === undefined)
      return false
    return pendingOperations.some(
      ({ operation }) =>
        operation.type === 'merge' &&
        operation.sessionId === session.id &&
        operation.number === identity.number &&
        operation.owner === identity.owner &&
        operation.repo === identity.repo,
    )
  }
  const sameMergeIdentity = (
    left: { number: number; owner?: string; repo?: string },
    right: { number: number; owner?: string; repo?: string },
  ) => {
    const leftIdentity = mergeIdentity(left)
    const rightIdentity = mergeIdentity(right)
    return (
      leftIdentity.owner !== undefined &&
      leftIdentity.repo !== undefined &&
      rightIdentity.owner !== undefined &&
      rightIdentity.repo !== undefined &&
      leftIdentity.number === rightIdentity.number &&
      leftIdentity.owner === rightIdentity.owner &&
      leftIdentity.repo === rightIdentity.repo
    )
  }
  const isMergeSuccessful = (target: PullRequest) =>
    mergeFeedback?.kind === 'success' &&
    sameMergeIdentity(mergeFeedback, target)
  const openMergeConfirmation = (target: PullRequest) => {
    const operation = mergeOperationFor(target)
    if (operation === null) return
    setMergeTarget(operation)
    setPullRequestListOpen(false)
    setMergeFeedback(null)
    setMergeConfirmationOpen(true)
  }
  const mergeAction =
    headerMergeOperation === null ||
    (mergeFeedback?.kind === 'success' &&
      sameMergeIdentity(mergeFeedback, headerMergeOperation)) ? null : (
      <PullRequestMergeAction
        merging={isMerging(headerMergeOperation)}
        onClick={() => {
          if (pullRequest === null) return
          openMergeConfirmation(pullRequest)
        }}
      />
    )
  const visibleMergeFeedback =
    mergeFeedback !== null &&
    pullRequests.some((candidate) =>
      sameMergeIdentity(mergeFeedback, candidate),
    )
      ? mergeFeedback
      : null
  const linkedTasks = getSessionTasks(session)
  const mobileRepoName = session.repo.split('/').at(-1) ?? session.repo

  const confirmMerge = async () => {
    if (mergeTarget === null || isMerging(mergeTarget)) return
    const mergeIdentity = {
      number: mergeTarget.number,
      owner: mergeTarget.owner,
      repo: mergeTarget.repo,
    }
    setMergeFeedback(null)
    setMergeConfirmationOpen(false)
    const result = await run(mergeTarget)
    result.match(
      (response) => {
        if (!isMergeResponse(response)) {
          setMergeFeedback({
            ...mergeIdentity,
            kind: 'error',
            message: 'The server returned an invalid merge response.',
          })
          return
        }
        const message =
          response.result === 'merged'
            ? `PR #${String(mergeTarget.number)} merged.`
            : response.result === 'alreadyMerged'
              ? `PR #${String(mergeTarget.number)} was already merged.`
              : `PR #${String(mergeTarget.number)} was added to the merge queue: ${response.message}`
        setMergeFeedback({
          ...mergeIdentity,
          kind: response.result === 'enqueued' ? 'queued' : 'success',
          message,
        })
      },
      (error) => {
        setMergeFeedback({
          ...mergeIdentity,
          kind: 'error',
          message: error.message,
        })
      },
    )
  }

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
    <header
      className={`conversation-header${isMobile && visibleMergeFeedback !== null ? ' has-session-pr-feedback' : ''}`}
    >
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
              onMerge={openMergeConfirmation}
              isMerging={isMerging}
              isMergeSuccessful={isMergeSuccessful}
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
              sessions={sessions}
              showTaskMenu={false}
              onMakePrimary={(taskId) => onAction('primary-task', { taskId })}
            />
            {session.task ? <span aria-hidden="true">·</span> : null}
            <span className="mobile-conversation-repo">{mobileRepoName}</span>
          </div>
          {visibleMergeFeedback !== null ? (
            <PullRequestMergeFeedback
              kind={visibleMergeFeedback.kind}
              message={visibleMergeFeedback.message}
              isMobile
            />
          ) : null}
        </>
      ) : (
        <>
          <div className="conversation-header-copy">
            <div className="conversation-context-line">
              <SessionTaskLinks
                task={session.task}
                tasks={linkedTasks}
                sessions={sessions}
                onMakePrimary={(taskId) => onAction('primary-task', { taskId })}
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
                  onMerge={openMergeConfirmation}
                  isMerging={isMerging}
                  isMergeSuccessful={isMergeSuccessful}
                  onOpenChange={setPullRequestListOpen}
                />
                {mergeAction}
                {visibleMergeFeedback === null ? null : (
                  <PullRequestMergeFeedback
                    kind={visibleMergeFeedback.kind}
                    message={visibleMergeFeedback.message}
                  />
                )}
                {session.category === 'done' && onClose !== undefined ? (
                  <Button
                    className="conversation-close cursor-pointer"
                    type="button"
                    variant="plain"
                    onClick={onClose}
                  >
                    Close
                  </Button>
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
      {mergeConfirmationOpen && mergeTarget !== null ? (
        <PullRequestMergeConfirmation
          number={mergeTarget.number}
          headSha={mergeTarget.expectedHeadSha}
          merging={isMerging(mergeTarget)}
          onCancel={() => {
            setMergeConfirmationOpen(false)
          }}
          onConfirm={() => {
            void confirmMerge()
          }}
        />
      ) : null}
    </header>
  )
}
