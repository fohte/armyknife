import '#components/session-tasks-panel.css'

import { useState } from 'react'

import type { SessionTask } from '#types'

const INITIAL_COMPLETED_TASKS = 4

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
              <li key={task.id}>
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
                        ? repo
                        : `P${String(task.priority)} · ${repo}`}
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
            {visibleCompletedTasks.map((task) => (
              <li key={task.id}>
                <a
                  className="session-tasks-panel-item session-tasks-panel-item--completed"
                  href={`https://tq.fohte.net/tasks/${encodeURIComponent(task.id)}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  <span className="session-tasks-panel-icon" aria-hidden="true">
                    ✓
                  </span>
                  <span className="session-tasks-panel-title">
                    <span className="session-task-link-id">
                      #{String(task.number)}
                    </span>{' '}
                    {task.title}
                  </span>
                </a>
                {primaryControl(task)}
              </li>
            ))}
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
