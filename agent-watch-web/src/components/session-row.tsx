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
import { useLayoutEffect, useRef, useState } from 'react'

import { useSessionElapsedStartedAt } from '#components/session-elapsed-context'
import { VibeIndicator } from '#components/vibe-indicator'
import { formatSessionDecisionCount } from '#session-decision-count'
import { sessionKindIcon } from '#session-kind-label'
import { formatLinkedTaskCount } from '#session-task-count'
import type { Session, SessionTask } from '#types'
import { useLiveElapsedOffsetSeconds } from '#use-live-elapsed-offset-seconds'

type SessionRowData = Pick<
  Session,
  | 'title'
  | 'category'
  | 'kind'
  | 'elapsedSeconds'
  | 'preview'
  | 'task'
  | 'repo'
  | 'branch'
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
}

function RepositoryLabel({ repository }: { repository: string }) {
  const repositoryRef = useRef<HTMLSpanElement>(null)
  const measureRef = useRef<HTMLSpanElement>(null)
  const [shortenOwner, setShortenOwner] = useState(false)
  const separator = repository.lastIndexOf('/')
  const isPlaceholder = repository === '—'
  const owner = separator < 0 ? '' : repository.slice(0, separator)
  const name = separator < 0 ? repository : repository.slice(separator + 1)

  useLayoutEffect(() => {
    const repositoryElement = repositoryRef.current
    const measureElement = measureRef.current
    if (
      repositoryElement === null ||
      measureElement === null ||
      owner.length === 0
    ) {
      return
    }

    let active = true
    const update = () => {
      if (!active) return
      const parentElement = repositoryElement.parentElement
      const branchElement =
        parentElement?.classList.contains('session-repo-line') === true
          ? parentElement.querySelector<HTMLElement>('.session-branch')
          : null
      const separatorElement =
        branchElement?.previousElementSibling instanceof HTMLElement
          ? branchElement.previousElementSibling
          : null
      const columnGap =
        parentElement === null
          ? 0
          : Number.parseFloat(getComputedStyle(parentElement).columnGap) || 0
      const branchMinimumWidth =
        branchElement === null
          ? 0
          : Number.parseFloat(getComputedStyle(branchElement).minWidth) || 0
      const availableWidth =
        parentElement?.classList.contains('session-repo-line') === true
          ? parentElement.clientWidth -
            branchMinimumWidth -
            (separatorElement?.getBoundingClientRect().width ?? 0) -
            (branchElement === null ? 0 : columnGap * 2)
          : repositoryElement.clientWidth
      const shouldShorten =
        measureElement.getBoundingClientRect().width > availableWidth + 1
      setShortenOwner(shouldShorten)
    }

    update()
    const observer = new ResizeObserver(update)
    observer.observe(repositoryElement)
    if (repositoryElement.parentElement !== null) {
      observer.observe(repositoryElement.parentElement)
    }
    document.fonts.addEventListener('loadingdone', update)
    return () => {
      active = false
      observer.disconnect()
      document.fonts.removeEventListener('loadingdone', update)
    }
  }, [owner, repository])

  return (
    <span
      ref={repositoryRef}
      className={`session-repo${isPlaceholder ? ' is-placeholder' : ''}`}
      title={repository}
    >
      <span className="session-repo-label">
        {owner.length > 0 ? (
          <>
            <span className="session-repo-owner">
              {shortenOwner ? '…' : owner}
            </span>
            <span aria-hidden="true">/</span>
          </>
        ) : null}
        <span className="session-repo-name">{name}</span>
      </span>
      {owner.length > 0 ? (
        <span
          ref={measureRef}
          aria-hidden="true"
          className="session-repo-measure"
        >
          {repository}
        </span>
      ) : null}
    </span>
  )
}

function BranchLabel({ branch }: { branch: string }) {
  return (
    <span className="session-branch" title={branch}>
      {branch}
    </span>
  )
}

function formatElapsedTime(totalSeconds: number) {
  if (totalSeconds < 60) return `${String(totalSeconds)}s`
  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) return `${String(minutes)}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${String(hours)}h`
  return `${String(Math.floor(hours / 24))}d`
}

function SessionAge({ elapsedSeconds }: { elapsedSeconds: number }) {
  const startedAt = useSessionElapsedStartedAt()
  if (startedAt === null) {
    return (
      <span className="session-age">{formatElapsedTime(elapsedSeconds)}</span>
    )
  }
  return (
    <LiveSessionAge elapsedSeconds={elapsedSeconds} startedAt={startedAt} />
  )
}

function LiveSessionAge({
  elapsedSeconds,
  startedAt,
}: {
  elapsedSeconds: number
  startedAt: number
}) {
  const elapsedOffsetSeconds = useLiveElapsedOffsetSeconds(startedAt)
  return (
    <span className="session-age">
      {formatElapsedTime(elapsedSeconds + elapsedOffsetSeconds)}
    </span>
  )
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
}: SessionRowProps) {
  const role = rolePresentation(session.workType)
  const RoleIcon = role.Icon
  const task = session.task
  const repo = session.repo.trim() || '—'
  const trimmedBranch = session.branch?.trim() ?? ''
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
  const rowPreview = [
    critReviewReason ?? decisionPreview ?? session.preview,
    todoLaterPreview,
  ]
    .filter((part) => part !== null && part.length > 0)
    .join(' · ')

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
          <SessionAge elapsedSeconds={session.elapsedSeconds} />
        </span>
        {density === 'full' && rowPreview.length > 0 ? (
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
              <RepositoryLabel repository={repo} />
            </>
          ) : null}
          {isCompact && decisionLabel !== null ? (
            <span className="session-decision-count">{decisionLabel}</span>
          ) : null}
        </span>
        {isCompact ? (
          <span className="session-repo-line">
            <RepositoryLabel repository={repo} />
            {trimmedBranch.length > 0 ? (
              <>
                <span className="session-branch-separator" aria-hidden="true">
                  ⑂
                </span>
                <BranchLabel branch={trimmedBranch} />
              </>
            ) : null}
          </span>
        ) : null}
      </span>
    </div>
  )
}
