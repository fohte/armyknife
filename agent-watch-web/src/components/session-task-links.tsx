import '#components/session-task-links.css'

import { useId, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'

import type { Session, SessionTask } from '#types'

type MenuPosition = {
  top: number
  left: number
  width: number
}

function SessionTaskId({
  priority,
  number,
}: Pick<SessionTask, 'priority' | 'number'>) {
  const label =
    priority === null
      ? `#${String(number)}`
      : `P${String(priority)} #${String(number)}`

  return <span className="session-task-link-id">{label}</span>
}

function getMenuPosition(summary: HTMLElement): MenuPosition {
  const summaryRect = summary.getBoundingClientRect()
  const paneRect = summary
    .closest('.conversation-pane')
    ?.getBoundingClientRect()
  const leftBound = Math.max(0, paneRect?.left ?? 0)
  const rightBound = Math.max(
    leftBound,
    Math.min(window.innerWidth, paneRect?.right ?? window.innerWidth),
  )
  const width = Math.min(420, rightBound - leftBound)
  const left = Math.min(
    Math.max(summaryRect.left, leftBound),
    rightBound - width,
  )

  return { top: summaryRect.bottom + 8, left, width }
}

export function SessionTaskLinks({
  task,
  tasks,
  sessions = [],
  onMakePrimary,
  taskListOpen,
}: {
  task: SessionTask | null
  tasks: SessionTask[]
  sessions?: readonly Session[]
  onMakePrimary?: (taskId: string) => Promise<boolean>
  taskListOpen?: boolean
}) {
  const taskMenuId = useId()
  const summaryRef = useRef<HTMLElement>(null)
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false)
  const [pendingTaskId, setPendingTaskId] = useState<string | null>(null)
  const [menuPosition, setMenuPosition] = useState<MenuPosition | null>(null)
  const menuOpen =
    task !== null && tasks.length > 1 && (taskListOpen ?? uncontrolledOpen)

  useLayoutEffect(() => {
    if (!menuOpen) return

    const summary = summaryRef.current
    if (summary === null) return

    const updatePosition = () => {
      const nextPosition = getMenuPosition(summary)
      setMenuPosition((currentPosition) =>
        currentPosition?.top === nextPosition.top &&
        currentPosition.left === nextPosition.left &&
        currentPosition.width === nextPosition.width
          ? currentPosition
          : nextPosition,
      )
    }
    const conversationPane = summary.closest('.conversation-pane')
    const resizeObserver =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(updatePosition)

    updatePosition()
    window.addEventListener('resize', updatePosition)
    document.addEventListener('scroll', updatePosition, true)
    resizeObserver?.observe(summary)
    if (conversationPane !== null) resizeObserver?.observe(conversationPane)

    return () => {
      window.removeEventListener('resize', updatePosition)
      document.removeEventListener('scroll', updatePosition, true)
      resizeObserver?.disconnect()
    }
  }, [menuOpen])

  if (task === null) return null

  const makePrimary = async (taskId: string) => {
    if (onMakePrimary === undefined) return
    setPendingTaskId(taskId)
    await onMakePrimary(taskId)
    setPendingTaskId(null)
  }

  const summaryByTaskId = new Map(
    tasks.map((linkedTask) => {
      const linkedSessions = sessions.filter((session) =>
        (session.tasks ?? (session.task === null ? [] : [session.task])).some(
          (sessionTask) => sessionTask.id === linkedTask.id,
        ),
      )
      const needsActionCount = linkedSessions.filter(
        (session) => session.category === 'needs_action',
      ).length
      const laterCount = linkedSessions.filter(
        (session) => session.category === 'later',
      ).length
      const status = [
        ...(needsActionCount > 0
          ? [`${String(needsActionCount)} need action`]
          : []),
        ...(laterCount > 0
          ? [
              linkedSessions.length === 1 && laterCount === 1
                ? 'Later'
                : `${String(laterCount)} Later`,
            ]
          : []),
      ].join(' · ')
      return [
        linkedTask.id,
        `${String(linkedSessions.length)} ${linkedSessions.length === 1 ? 'session' : 'sessions'}${status === '' ? '' : ` · ${status}`}`,
      ]
    }),
  )

  return (
    <>
      <div className="session-task-line">
        <a
          className="conversation-task-link"
          href={`https://tq.fohte.net/tasks/${encodeURIComponent(task.id)}`}
          target="_blank"
          rel="noreferrer"
        >
          <SessionTaskId priority={task.priority} number={task.number} />{' '}
          {task.title}
        </a>
        {tasks.length > 1 ? (
          <details
            className="session-task-links"
            aria-owns={taskMenuId}
            open={menuOpen}
            onToggle={(event) => {
              const open = event.currentTarget.open
              if (taskListOpen === undefined) setUncontrolledOpen(open)
              if (!open) setMenuPosition(null)
            }}
          >
            <summary
              ref={summaryRef}
              aria-controls={taskMenuId}
              aria-label={`${String(tasks.length - 1)} more linked tq tasks`}
            >
              +{tasks.length - 1} tasks ▾
            </summary>
          </details>
        ) : null}
      </div>
      {tasks.length > 1
        ? createPortal(
            <div
              id={taskMenuId}
              className="session-task-links-menu"
              role="region"
              aria-label="Linked tq tasks"
              hidden={!menuOpen || menuPosition === null}
              style={
                menuPosition === null
                  ? undefined
                  : {
                      top: menuPosition.top,
                      left: menuPosition.left,
                      width: menuPosition.width,
                    }
              }
            >
              <div className="session-task-links-heading">
                Linked tq tasks · Inbox and Board use the primary one
              </div>
              <ul>
                {tasks.map((linkedTask) => {
                  const isPrimary = linkedTask.id === task.id

                  return (
                    <li
                      key={linkedTask.id}
                      className={
                        isPrimary
                          ? 'session-task-links-item session-task-links-item--primary'
                          : 'session-task-links-item'
                      }
                    >
                      <span
                        className="session-task-links-check"
                        aria-hidden="true"
                      >
                        {isPrimary ? '✓' : null}
                      </span>
                      <div className="session-task-links-copy">
                        <a
                          href={`https://tq.fohte.net/tasks/${encodeURIComponent(linkedTask.id)}`}
                          target="_blank"
                          rel="noreferrer"
                        >
                          <SessionTaskId
                            priority={linkedTask.priority}
                            number={linkedTask.number}
                          />{' '}
                          {linkedTask.title}
                        </a>
                        <span className="session-task-links-summary">
                          {summaryByTaskId.get(linkedTask.id)}
                        </span>
                      </div>
                      {isPrimary ? (
                        <span className="session-task-links-primary">
                          Primary
                        </span>
                      ) : (
                        <button
                          className="session-task-links-make-primary"
                          type="button"
                          disabled={pendingTaskId !== null}
                          onClick={() => void makePrimary(linkedTask.id)}
                        >
                          Make primary
                        </button>
                      )}
                      {linkedTask.isClosed === true ? (
                        <span className="session-task-completed">
                          Completed
                        </span>
                      ) : null}
                    </li>
                  )
                })}
              </ul>
            </div>,
            document.body,
          )
        : null}
    </>
  )
}
