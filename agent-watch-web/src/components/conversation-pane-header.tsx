import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import { ChevronLeft, PanelRightClose, PanelRightOpen } from 'lucide-react'

import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { SessionHeading } from '#components/session-heading'
import { SessionTaskLinks } from '#components/session-task-links'
import { StarredSessionToggle } from '#components/starred-session-toggle'
import type {
  BackgroundTask,
  OpenInTerminalStatus,
  Session,
  SessionActions,
} from '#types'

type ConversationPaneHeaderProps = {
  session: Session
  sessions: readonly Session[]
  actions: SessionActions | null
  isMobile: boolean
  statusPanelOpen: boolean
  backgroundTasks: readonly BackgroundTask[]
  openInTerminalStatus: OpenInTerminalStatus
  isToday: boolean
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
  onAction,
  onOpenInTerminal,
  onSelectSession,
  onToggleStatusPanel,
  onToggleToday,
  onBack,
  onLater,
  onClose,
}: ConversationPaneHeaderProps) {
  const parent = session.parent
  const linkedTasks =
    session.tasks ?? (session.task === null ? [] : [session.task])

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
            <SessionHeading
              title={session.title}
              isPaused={session.isPaused}
              sessionId={session.id}
            />
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
                {session.pullRequest ? (
                  <a
                    className="conversation-pr-link"
                    href={`https://github.com/${session.repo}/pull/${String(session.pullRequest.number)}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    #{session.pullRequest.number} PR
                  </a>
                ) : null}
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
            <SessionHeading
              title={session.title}
              isPaused={session.isPaused}
              sessionId={session.id}
            />
          </div>
        </>
      )}
    </header>
  )
}
