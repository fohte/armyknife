import { SessionPausedBadge } from '#components/session-paused-badge'
import type { SessionTask } from '#types'

export function SessionHeading({
  title,
  isPaused,
  task,
  tasks,
  taskListOpen,
}: {
  title: string
  isPaused: boolean
  task: SessionTask | null
  tasks: SessionTask[]
  taskListOpen?: boolean
}) {
  return (
    <>
      {task !== null ? (
        <div className="session-heading-task-line">
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
                +{tasks.length - 1}
              </summary>
              <ul>
                {tasks.map((linkedTask) => (
                  <li key={linkedTask.id}>
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
                    {linkedTask.isClosed === true ? (
                      <span className="session-task-completed">Completed</span>
                    ) : null}
                  </li>
                ))}
              </ul>
            </details>
          ) : null}
        </div>
      ) : null}
      <h1>{title || 'Untitled session'}</h1>
      <SessionPausedBadge isPaused={isPaused} />
    </>
  )
}
