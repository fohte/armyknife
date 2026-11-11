import '#components/session-task-links.css'

import type { SessionTask } from '#types'

export function SessionTaskLinks({
  task,
  tasks,
  taskListOpen,
}: {
  task: SessionTask | null
  tasks: SessionTask[]
  taskListOpen?: boolean
}) {
  if (task === null) return null

  return (
    <div className="session-task-line" data-session-task-links-root="">
      <a
        className="conversation-task-link"
        href={`https://tq.fohte.net/tasks/${encodeURIComponent(task.id)}`}
        target="_blank"
        rel="noreferrer"
      >
        {task.priority !== null ? <span>P{task.priority}</span> : null} #
        {task.number} {task.title}
      </a>
      {tasks.length > 1 ? (
        <details
          className="session-task-links"
          open={taskListOpen}
          data-task-links=""
        >
          <summary
            aria-label={`${String(tasks.length - 1)} more linked tq tasks`}
          >
            +{tasks.length - 1} tasks ▾
          </summary>
          <div
            className="session-task-links-menu"
            role="dialog"
            aria-label="Linked tq tasks"
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
                    className="session-task-links-item"
                    data-primary={isPrimary}
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
                      {linkedTask.priority !== null ? (
                        <span>P{linkedTask.priority} </span>
                      ) : null}
                      #{linkedTask.number} {linkedTask.title}
                    </a>
                    {isPrimary ? (
                      <span className="session-task-links-primary">
                        Primary
                      </span>
                    ) : linkedTask.isClosed === true ? (
                      <span className="session-task-completed">Completed</span>
                    ) : null}
                  </li>
                )
              })}
            </ul>
          </div>
        </details>
      ) : null}
    </div>
  )
}
