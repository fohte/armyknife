import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import {
  ChevronLeft,
  PanelRightClose,
  PanelRightOpen,
  Pencil,
} from 'lucide-react'
import { type SubmitEvent, useState } from 'react'

import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { PullRequestMergeConfirmation } from '#components/pull-request-merge-confirmation'
import { SessionHeading } from '#components/session-heading'
import { SessionTaskLinks } from '#components/session-task-links'
import { StarredSessionToggle } from '#components/starred-session-toggle'
import { useSessionOperations } from '#session-operation-context'
import type {
  BackgroundTask,
  OpenInTerminalStatus,
  Session,
  SessionActions,
} from '#types'

type MergeResponse =
  | { result: 'merged' }
  | { result: 'alreadyMerged' }
  | { result: 'enqueued'; message: string }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

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
  sessions: readonly Session[]
  actions: SessionActions | null
  isMobile: boolean
  statusPanelOpen: boolean
  backgroundTasks: readonly BackgroundTask[]
  openInTerminalStatus: OpenInTerminalStatus
  isToday: boolean
  titleEditing?: boolean
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
  sessions,
  actions,
  isMobile,
  statusPanelOpen,
  backgroundTasks,
  openInTerminalStatus,
  isToday,
  titleEditing = false,
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
  const [mergeFeedback, setMergeFeedback] = useState<{
    number: number
    kind: 'success' | 'queued' | 'error'
    message: string
  } | null>(null)
  const { pendingOperations, run } = useSessionOperations()
  const parent = session.parent
  const pullRequest = session.pullRequest
  const mergeOperation =
    pullRequest?.state === 'Open' &&
    pullRequest.owner !== undefined &&
    pullRequest.repo !== undefined &&
    pullRequest.headSha !== null &&
    pullRequest.headSha !== undefined
      ? {
          type: 'merge' as const,
          sessionId: session.id,
          number: pullRequest.number,
          owner: pullRequest.owner,
          repo: pullRequest.repo,
          expectedHeadSha: pullRequest.headSha,
        }
      : null
  const merging =
    mergeOperation !== null &&
    pendingOperations.some(
      ({ operation }) =>
        operation.type === 'merge' &&
        operation.sessionId === session.id &&
        operation.number === mergeOperation.number,
    )
  const linkedTasks =
    session.tasks ?? (session.task === null ? [] : [session.task])

  const confirmMerge = async () => {
    if (mergeOperation === null || merging) return
    setMergeFeedback(null)
    setMergeConfirmationOpen(false)
    const result = await run(mergeOperation)
    result.match(
      (response) => {
        if (!isMergeResponse(response)) {
          setMergeFeedback({
            number: mergeOperation.number,
            kind: 'error',
            message: 'The server returned an invalid merge response.',
          })
          return
        }
        const message =
          response.result === 'merged'
            ? `PR #${String(mergeOperation.number)} merged.`
            : response.result === 'alreadyMerged'
              ? `PR #${String(mergeOperation.number)} was already merged.`
              : `PR #${String(mergeOperation.number)} was added to the merge queue: ${response.message}`
        setMergeFeedback({
          number: mergeOperation.number,
          kind: response.result === 'enqueued' ? 'queued' : 'success',
          message,
        })
      },
      (error) => {
        setMergeFeedback({
          number: mergeOperation.number,
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
        isPaused={session.isPaused}
        sessionId={session.id}
      />
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
          <div className="mobile-conversation-heading">
            {titleHeading}
            <div className="mobile-conversation-context">
              <SessionTaskLinks
                task={session.task}
                tasks={linkedTasks}
                sessions={sessions}
                onMakePrimary={(taskId) => onAction('primary-task', { taskId })}
              />
              {session.task ? <span aria-hidden="true">·</span> : null}
              <span>{session.repo.split('/').at(-1)}</span>
            </div>
          </div>
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
            {backgroundTasks.length > 0 ? (
              <span className="conversation-status-panel-count">
                {backgroundTasks.length}
              </span>
            ) : null}
          </Button>
          <Button
            className="mobile-conversation-later cursor-pointer"
            type="button"
            variant="plain"
            onClick={onLater}
          >
            Later
          </Button>
          {session.category === 'done' && onClose !== undefined ? (
            <Button
              className="mobile-conversation-close cursor-pointer"
              type="button"
              variant="plain"
              onClick={onClose}
            >
              Close
            </Button>
          ) : null}
          <StarredSessionToggle
            className="mobile-conversation-today cursor-pointer"
            size="mobile"
            isStarred={isToday}
            onToggleToday={onToggleToday}
          />
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
                {pullRequest ? (
                  <a
                    className="conversation-pr-link"
                    href={
                      pullRequest.url ??
                      `https://github.com/${session.repo}/pull/${String(pullRequest.number)}`
                    }
                    target="_blank"
                    rel="noreferrer"
                  >
                    #{pullRequest.number} PR
                  </a>
                ) : null}
                {mergeOperation === null ||
                (mergeFeedback?.kind === 'success' &&
                  mergeFeedback.number === mergeOperation.number) ? null : (
                  <Button
                    className="conversation-pr-merge-button"
                    type="button"
                    variant="plain"
                    disabled={merging}
                    onClick={() => {
                      setMergeFeedback(null)
                      setMergeConfirmationOpen(true)
                    }}
                  >
                    {merging ? 'Merging…' : 'Merge'}
                  </Button>
                )}
                {mergeFeedback === null ||
                mergeFeedback.number !== pullRequest?.number ? null : (
                  <span
                    className={`conversation-pr-merge-feedback conversation-pr-merge-feedback--${mergeFeedback.kind}`}
                    role={mergeFeedback.kind === 'error' ? 'alert' : 'status'}
                  >
                    {mergeFeedback.message}
                  </span>
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
      {mergeConfirmationOpen && mergeOperation !== null ? (
        <PullRequestMergeConfirmation
          number={mergeOperation.number}
          headSha={mergeOperation.expectedHeadSha}
          merging={merging}
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
