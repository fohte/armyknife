import '#components/session-task-links.css'

import { useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'

import type { SessionTask } from '#types'

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
  taskListOpen,
}: {
  task: SessionTask | null
  tasks: SessionTask[]
  taskListOpen?: boolean
}) {
  const summaryRef = useRef<HTMLElement>(null)
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false)
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
            open={menuOpen}
            onToggle={(event) => {
              const open = event.currentTarget.open
              if (taskListOpen === undefined) setUncontrolledOpen(open)
              if (!open) setMenuPosition(null)
            }}
          >
            <summary
              ref={summaryRef}
              aria-label={`${String(tasks.length - 1)} more linked tq tasks`}
            >
              +{tasks.length - 1} tasks ▾
            </summary>
          </details>
        ) : null}
      </div>
      {menuOpen && menuPosition !== null
        ? createPortal(
            <div
              className="session-task-links-menu"
              style={{
                top: menuPosition.top,
                left: menuPosition.left,
                width: menuPosition.width,
              }}
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
                      {isPrimary ? (
                        <span className="session-task-links-primary">
                          Primary
                        </span>
                      ) : null}
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
