import '#components/session-row.css'

import {
  Bug,
  Code2,
  Ellipsis,
  Hourglass,
  Palette,
  Search,
  UsersRound,
  Workflow,
} from 'lucide-react'
import { useLayoutEffect, useRef, useState } from 'react'

import { useSessionElapsedStartedAt } from '#components/session-elapsed-context'
import { VibeIndicator } from '#components/vibe-indicator'
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
  showWaitingReason?: boolean
}

function RepositoryLabel({ repository }: { repository: string }) {
  const repositoryRef = useRef<HTMLSpanElement>(null)
  const measureRef = useRef<HTMLSpanElement>(null)
  const clippedAtAvailableWidthRef = useRef<number | null>(null)
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

    clippedAtAvailableWidthRef.current = null
    const ownerElement = repositoryElement.querySelector<HTMLElement>(
      '.session-repo-owner',
    )
    const ownerSeparatorElement = repositoryElement.querySelector<HTMLElement>(
      '.session-repo-owner-separator',
    )
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
      const parentStyle =
        parentElement === null ? null : getComputedStyle(parentElement)
      const parentPadding =
        parentStyle === null
          ? 0
          : (Number.parseFloat(parentStyle.paddingLeft) || 0) +
            (Number.parseFloat(parentStyle.paddingRight) || 0)
      const isRepositoryLine =
        parentElement?.classList.contains('session-repo-line') === true
      const isMetaLine =
        parentElement?.classList.contains('session-meta-line') === true
      const ownerPrefixIsClipped =
        (ownerElement !== null &&
          ownerElement.scrollWidth > ownerElement.clientWidth) ||
        (ownerSeparatorElement !== null &&
          ownerSeparatorElement.scrollWidth > ownerSeparatorElement.clientWidth)
      const metaSiblings = isMetaLine
        ? Array.from(parentElement.children).filter(
            (child): child is HTMLElement =>
              child instanceof HTMLElement && child !== repositoryElement,
          )
        : []
      const metaSiblingWidth = metaSiblings.reduce((width, sibling) => {
        if (sibling.classList.contains('session-task')) {
          const taskTitle = sibling.querySelector<HTMLElement>(
            '.session-task-title',
          )
          const taskReference = sibling.querySelector<HTMLElement>(
            '.session-task-reference',
          )
          if (taskTitle !== null && taskReference !== null) {
            const taskGap =
              Number.parseFloat(getComputedStyle(sibling).columnGap) || 0
            return (
              width +
              taskTitle.scrollWidth +
              taskReference.getBoundingClientRect().width +
              taskGap
            )
          }
        }
        return width + Math.max(sibling.clientWidth, sibling.scrollWidth)
      }, 0)
      const metaGapCount = metaSiblings.length
      const availableWidth = isRepositoryLine
        ? parentElement.clientWidth -
          parentPadding -
          branchMinimumWidth -
          (separatorElement?.getBoundingClientRect().width ?? 0) -
          (branchElement === null ? 0 : columnGap * 2)
        : isMetaLine
          ? parentElement.clientWidth -
            parentPadding -
            metaSiblingWidth -
            columnGap * metaGapCount
          : repositoryElement.clientWidth
      if (ownerPrefixIsClipped) {
        clippedAtAvailableWidthRef.current = Math.max(
          clippedAtAvailableWidthRef.current ?? 0,
          availableWidth,
        )
      }
      const isOwnerShortened =
        repositoryElement.classList.contains('is-owner-shortened')
      const isStillAtClippedWidth =
        isOwnerShortened &&
        clippedAtAvailableWidthRef.current !== null &&
        availableWidth <= clippedAtAvailableWidthRef.current
      const repositoryWidth = measureElement.getBoundingClientRect().width
      const shouldShorten =
        ownerPrefixIsClipped ||
        repositoryWidth > availableWidth + (isOwnerShortened ? 0 : 1) ||
        isStillAtClippedWidth
      if (!shouldShorten) clippedAtAvailableWidthRef.current = null
      setShortenOwner(shouldShorten)
    }

    update()
    const observer = new ResizeObserver(update)
    observer.observe(repositoryElement)
    const lineElement = repositoryElement.parentElement
    if (lineElement !== null) {
      observer.observe(lineElement)
      for (const sibling of lineElement.children) {
        if (sibling instanceof HTMLElement && sibling !== repositoryElement) {
          observer.observe(sibling)
        }
      }
    }
    if (ownerElement !== null) observer.observe(ownerElement)
    if (ownerSeparatorElement !== null) observer.observe(ownerSeparatorElement)
    const rowElement = repositoryElement.closest('.session-row-content')
    if (rowElement !== null) {
      observer.observe(rowElement)
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
      className={`session-repo${isPlaceholder ? ' is-placeholder' : ''}${shortenOwner ? ' is-owner-shortened' : ''}`}
      title={repository}
    >
      <span className="session-repo-label">
        {owner.length > 0 ? (
          <>
            <span className="session-repo-owner">
              {shortenOwner ? '…' : owner}
            </span>
            <span aria-hidden="true" className="session-repo-owner-separator">
              /
            </span>
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
  if (session.category === 'waiting') {
    return <Hourglass aria-hidden="true" size={14} />
  }
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
  showWaitingReason = false,
}: SessionRowProps) {
  const role = rolePresentation(session.workType)
  const RoleIcon = role.Icon
  const task = session.task
  const repo = session.repo.trim() || '—'
  const trimmedBranch = session.branch?.trim() ?? ''
  const isCompact = density !== 'full'
  const linkedTaskCount = isCompact ? formatLinkedTaskCount(session.tasks) : ''
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
    critReviewReason ??
      (session.category === 'waiting' &&
      (density === 'full' || showWaitingReason)
        ? session.waitingReason
        : null) ??
      session.preview,
    todoLaterPreview,
  ]
    .filter((part) => part !== null && part.length > 0)
    .join(' · ')
  const showPreview =
    density === 'full' || (session.category === 'waiting' && showWaitingReason)

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
            : session.category === 'waiting'
              ? 'Waiting'
              : session.category === 'done'
                ? 'Done'
                : (session.kind ?? 'Stopped')
        }
        title={
          session.category === 'in_progress'
            ? 'Working'
            : session.category === 'waiting'
              ? 'Waiting'
              : undefined
        }
      >
        {statusIcon(session)}
      </span>
      <span className="session-content">
        <span className="session-title-line">
          <span
            className={`session-title${session.unread === true ? ' is-unread' : ''}`}
            title={session.title}
          >
            {session.title}
          </span>
          {session.vibeMode === undefined ? null : (
            <VibeIndicator mode={session.vibeMode} />
          )}
          <SessionAge elapsedSeconds={session.elapsedSeconds} />
        </span>
        {showPreview && rowPreview.length > 0 ? (
          <span className="session-preview">{rowPreview}</span>
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
          <span
            className="session-task"
            title={
              task === null
                ? undefined
                : `${taskReference(task)} ${task.title}${linkedTaskCount}`
            }
          >
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
