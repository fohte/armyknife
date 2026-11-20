import '#components/session-task-links.css'

import type { SessionTask } from '#types'

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

export function SessionTaskLinks({
  task,
  tasks,
  onOpenTasks,
}: {
  task: SessionTask | null
  tasks: readonly SessionTask[]
  onOpenTasks: () => void
}) {
  if (task === null) return null

  const selectableTasks = tasks.filter(
    (linkedTask) => linkedTask.id === task.id || linkedTask.isClosed !== true,
  )

  const additionalTaskCount = selectableTasks.length - 1

  return (
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
      {additionalTaskCount > 0 ? (
        <button
          className="session-task-links"
          type="button"
          aria-label={`Open ${String(additionalTaskCount)} linked tq ${additionalTaskCount === 1 ? 'task' : 'tasks'}`}
          onClick={() => {
            onOpenTasks()
          }}
        >
          +{String(additionalTaskCount)}
        </button>
      ) : null}
    </div>
  )
}
