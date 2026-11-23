import '#components/session-row.css'

import {
  Bug,
  Code2,
  Ellipsis,
  Palette,
  Search,
  UsersRound,
  Workflow,
} from 'lucide-react'

import { VibeIndicator } from '#components/vibe-indicator'
import { formatSessionDecisionCount } from '#session-decision-count'
import { sessionKindIcon } from '#session-kind-label'
import { formatLinkedTaskCount } from '#session-task-count'
import type { Session, SessionTask } from '#types'

type SessionRowData = Pick<
  Session,
  'title' | 'category' | 'kind' | 'elapsedSeconds' | 'preview' | 'task' | 'repo'
> &
  Partial<
    Pick<
      Session,
      | 'engine'
      | 'workType'
      | 'unread'
      | 'tasks'
      | 'decisionCount'
      | 'unreviewedDecisionCount'
      | 'pullRequest'
      | 'vibeMode'
      | 'isWaitingForCritReview'
      | 'waitingReason'
      | 'todoLaterCount'
      | 'todoLaterTitle'
    >
  >

type SessionRowProps = {
  session: SessionRowData
  density?: 'full' | 'compact' | 'compact-phone'
  isMobile?: boolean
  branch?: string | null
}

function formatElapsedTime(totalSeconds: number) {
  if (totalSeconds < 60) return `${String(totalSeconds)}s`
  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) return `${String(minutes)}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h`
  return `${String(Math.floor(hours / 24))}d`
}

function rolePresentation(workType: string | null | undefined) {
  const role = workType?.trim() ?? ''
  const normalized = role.toLowerCase().replaceAll('_', '-')
  switch (normalized) {
    case 'development':
      return { label: role, Icon: Code2 }
    case 'design':
      return { label: role, Icon: Workflow }
    case 'designer':
      return { label: role, Icon: Palette }
    case 'research':
      return { label: role, Icon: Search }
    case 'debug-flow':
      return { label: role, Icon: Bug }
    case 'pm':
      return { label: role, Icon: UsersRound }
    default:
      return role.length === 0
        ? { label: 'No role', Icon: Ellipsis }
        : { label: role, Icon: Ellipsis }
  }
}

function taskReference(task: SessionTask) {
  return `${task.priority === null ? '—' : `P${String(task.priority)}`} #${String(task.number)}`
}

function statusIcon(session: SessionRowData) {
  if (session.category === 'in_progress') {
    return <span className="session-row-working-dot" />
  }
  if (session.category === 'done') return <span aria-hidden="true">✓</span>

  const kind = session.kind ?? 'Stopped'
  const Icon = sessionKindIcon(kind)
  return (
    <Icon
      aria-hidden="true"
      className={`session-row-kind-icon session-row-kind-icon--${kind.toLowerCase()}`}
      size={14}
    />
  )
}

export function SessionRow({
  session,
  density = 'full',
  isMobile = false,
  branch,
}: SessionRowProps) {
  const role = rolePresentation(session.workType)
  const RoleIcon = role.Icon
  const task = session.task
  const repo = session.repo.trim() || '—'
  const trimmedBranch = branch?.trim() ?? ''
  const isCompact = density !== 'full'
  const linkedTaskCount = isCompact ? formatLinkedTaskCount(session.tasks) : ''
  const decisionLabel = formatSessionDecisionCount(session)
  const decisionPreview =
    session.category === 'done' && (session.unreviewedDecisionCount ?? 0) > 0
      ? decisionLabel
      : null
  const critReviewReason =
    session.isWaitingForCritReview === true &&
    session.kind === 'Review' &&
    (session.waitingReason?.trim().length ?? 0) > 0
      ? session.waitingReason
      : null
  const todoLaterPreview =
    (session.todoLaterCount ?? 0) > 0 && !isMobile
      ? ['To-do · later', session.todoLaterTitle]
          .filter(
            (part) => part !== null && part !== undefined && part.length > 0,
          )
          .join(' · ')
      : null
  const preview = critReviewReason ?? decisionPreview ?? session.preview
  const rowPreview = todoLaterPreview ?? preview

  return (
    <div
      className={`session-row-content density-${density}${isMobile ? ' is-mobile' : ''}`}
      data-session-row-content=""
    >
      <span
        className={`session-row-status category-${session.category}`}
        role="img"
        aria-label={
          session.category === 'in_progress'
            ? 'Working'
            : session.category === 'done'
              ? 'Done'
              : (session.kind ?? 'Stopped')
        }
        title={session.category === 'in_progress' ? 'Working' : undefined}
      >
        {statusIcon(session)}
      </span>
      <span className="session-content">
        <span className="session-title-line">
          <span
            className={`session-title${session.unread === true ? ' is-unread' : ''}`}
          >
            {session.title}
          </span>
          {session.vibeMode === undefined ? null : (
            <VibeIndicator mode={session.vibeMode} />
          )}
          <span className="session-age">
            {formatElapsedTime(session.elapsedSeconds)}
          </span>
        </span>
        {density === 'full' && rowPreview !== null && rowPreview.length > 0 ? (
          <span
            className={`session-preview${decisionPreview === null ? '' : ' is-decision-preview'}`}
          >
            {rowPreview}
          </span>
        ) : null}
        <span className="session-meta-line">
          <span
            className="session-role-icon"
            role="img"
            aria-label={role.label}
            title={role.label}
          >
            <RoleIcon aria-hidden="true" size={13} />
          </span>
          {session.engine ? (
            <>
              <span className="session-engine">{session.engine}</span>
              <span className="session-meta-separator" aria-hidden="true">
                ·
              </span>
            </>
          ) : null}
          <span className="session-task">
            {task === null ? (
              '— No task'
            ) : (
              <>
                <span className="session-task-reference">
                  {taskReference(task)}
                </span>{' '}
                <span className="session-task-title">
                  {task.title}
                  {linkedTaskCount}
                </span>
              </>
            )}
          </span>
          {!isCompact ? (
            <>
              <span className="session-repo-separator" aria-hidden="true">
                ·
              </span>
              <span className="session-repo">{repo}</span>
            </>
          ) : null}
          {isCompact && decisionLabel !== null ? (
            <span className="session-decision-count">{decisionLabel}</span>
          ) : null}
        </span>
        {isCompact ? (
          <span className="session-repo-line">
            <span className="session-repo">{repo}</span>
            {trimmedBranch.length > 0 ? (
              <>
                <span className="session-repo-separator" aria-hidden="true">
                  ·
                </span>
                <span className="session-branch">{trimmedBranch}</span>
              </>
            ) : null}
          </span>
        ) : null}
      </span>
    </div>
  )
}
