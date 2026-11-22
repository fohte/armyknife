import '#components/session-tasks-panel.css'

import { useState } from 'react'

import type { SessionTask } from '#types'

const INITIAL_COMPLETED_TASKS = 4

function formatTaskAge(updatedAt: string) {
  const updatedAtMs = Date.parse(updatedAt)
  if (Number.isNaN(updatedAtMs)) return null
  const elapsedMinutes = Math.max(
    0,
    Math.floor((Date.now() - updatedAtMs) / 60_000),
  )
  if (elapsedMinutes < 60) return `${String(elapsedMinutes)}m`
  const elapsedHours = Math.floor(elapsedMinutes / 60)
  if (elapsedHours < 24) return `${String(elapsedHours)}h`
  return `${String(Math.floor(elapsedHours / 24))}d`
}

export function SessionTasksPanel({
  tasks,
  repo,
  primaryTaskId,
  onMakePrimary,
}: {
  tasks: readonly SessionTask[]
  repo: string
  primaryTaskId: string | null
  onMakePrimary?: (taskId: string) => Promise<boolean>
}) {
  const [showAllCompleted, setShowAllCompleted] = useState(false)
  const [pendingTaskId, setPendingTaskId] = useState<string | null>(null)
  const inProgressTasks = tasks.filter((task) => task.isClosed !== true)
  const completedTasks = tasks.filter((task) => task.isClosed === true)
  const visibleCompletedTasks = showAllCompleted
    ? completedTasks
    : completedTasks.slice(0, INITIAL_COMPLETED_TASKS)
  const remainingCompletedCount =
    completedTasks.length - visibleCompletedTasks.length
  const shortRepo = repo.split('/').at(-1) ?? repo

  const makePrimary = async (taskId: string) => {
    if (onMakePrimary === undefined) return
    setPendingTaskId(taskId)
    await onMakePrimary(taskId)
    setPendingTaskId(null)
  }

  const primaryControl = (task: SessionTask) => {
    if (onMakePrimary === undefined) return null
    return task.id === primaryTaskId ? (
      <span className="session-tasks-panel-primary">Primary</span>
    ) : (
      <button
        className="session-tasks-panel-primary-action"
        type="button"
        disabled={pendingTaskId !== null}
        onClick={() => void makePrimary(task.id)}
      >
        Make primary
      </button>
    )
  }

  return (
    <div className="session-tasks-panel">
      {inProgressTasks.length === 0 ? null : (
        <section aria-labelledby="session-tasks-in-progress-heading">
          <h3
            id="session-tasks-in-progress-heading"
            className="session-tasks-panel-heading"
          >
            IN PROGRESS
          </h3>
          <ul className="session-tasks-panel-list">
            {inProgressTasks.map((task) => (
              <li
                key={task.id}
                className="session-tasks-panel-row session-tasks-panel-row--open"
              >
                <a
                  className="session-tasks-panel-item session-tasks-panel-item--open"
                  href={`https://tq.fohte.net/tasks/${encodeURIComponent(task.id)}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  <span className="session-tasks-panel-icon" aria-hidden="true">
                    ●
                  </span>
                  <span className="session-tasks-panel-copy">
                    <span className="session-tasks-panel-title">
                      <span className="session-task-link-id">
                        #{String(task.number)}
                      </span>{' '}
                      {task.title}
                    </span>
                    <span className="session-tasks-panel-meta">
                      {task.priority === null
                        ? shortRepo
                        : `P${String(task.priority)} · ${shortRepo}`}
                    </span>
                  </span>
                </a>
                {primaryControl(task)}
              </li>
            ))}
          </ul>
        </section>
      )}
      {completedTasks.length === 0 ? null : (
        <section aria-labelledby="session-tasks-completed-heading">
          <h3
            id="session-tasks-completed-heading"
            className="session-tasks-panel-heading session-tasks-panel-heading--completed"
          >
            COMPLETED <span>{completedTasks.length}</span>
          </h3>
          <ul className="session-tasks-panel-list">
            {visibleCompletedTasks.map((task) => {
              const age =
                task.updatedAt == null ? null : formatTaskAge(task.updatedAt)
              return (
                <li
                  key={task.id}
                  className="session-tasks-panel-row session-tasks-panel-row--completed"
                >
                  <a
                    className="session-tasks-panel-item session-tasks-panel-item--completed"
                    href={`https://tq.fohte.net/tasks/${encodeURIComponent(task.id)}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    <span
                      className="session-tasks-panel-icon"
                      aria-hidden="true"
                    >
                      ✓
                    </span>
                    <span className="session-tasks-panel-copy">
                      <span className="session-tasks-panel-title">
                        <span className="session-task-link-id">
                          #{String(task.number)}
                        </span>{' '}
                        {task.title}
                      </span>
                      {age === null ? null : ' '}
                      {age === null ? null : (
                        <time
                          className="session-tasks-panel-age"
                          dateTime={task.updatedAt ?? undefined}
                        >
                          {age}
                        </time>
                      )}
                    </span>
                  </a>
                  {primaryControl(task)}
                </li>
              )
            })}
          </ul>
          {remainingCompletedCount === 0 ? null : (
            <button
              className="session-tasks-panel-show-more"
              type="button"
              onClick={() => {
                setShowAllCompleted(true)
              }}
            >
              Show {remainingCompletedCount} more
            </button>
          )}
        </section>
      )}
    </div>
  )
}
