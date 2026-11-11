import '#components/running-tasks-panel.css'

import { useEffect, useId, useState } from 'react'

import { requestText } from '#components/conversation-requests'
import { formatElapsedTime } from '#components/format-elapsed-time'
import { BackgroundTaskOutput } from '#components/running-task-output'
import type { BackgroundTask } from '#types'

type RunningTasksPanelProps = {
  sessionId: string
  tasks: readonly BackgroundTask[]
}

export function RunningTasksPanel({
  sessionId,
  tasks,
}: RunningTasksPanelProps) {
  const subagents = tasks.filter((task) => task.kind === 'subagent')
  const backgroundTasks = tasks.filter(
    (task) => task.kind === 'background_task',
  )

  return (
    <div className="running-tasks-panel">
      {subagents.length > 0 ? (
        <section className="running-tasks-group" aria-label="Subagents">
          <h2>SUBAGENTS</h2>
          <ul>
            {subagents.map((task) => (
              <li className="running-task" key={task.id}>
                <span className="running-task-icon" aria-hidden="true">
                  ◐
                </span>
                <div className="running-task-content">
                  <div className="running-task-subagent-heading">
                    <span
                      className="running-task-agent-type"
                      title={task.agentType ?? 'Agent'}
                    >
                      {task.agentType ?? 'Agent'}
                    </span>
                    <span
                      className="running-task-subagent-description"
                      title={task.description}
                    >
                      {task.description}
                    </span>
                  </div>
                  <span className="running-task-elapsed">
                    {formatElapsedTime(task.elapsedSeconds)}
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      {backgroundTasks.length > 0 ? (
        <section className="running-tasks-group" aria-label="Background tasks">
          <h2>BACKGROUND TASKS</h2>
          <ul>
            {backgroundTasks.map((task) => (
              <li className="running-task" key={task.id}>
                <span className="running-task-icon" aria-hidden="true">
                  ▶
                </span>
                <BackgroundTaskRow sessionId={sessionId} task={task} />
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  )
}

function BackgroundTaskRow({
  sessionId,
  task,
}: {
  sessionId: string
  task: BackgroundTask
}) {
  const outputId = useId()
  const [isOpen, setIsOpen] = useState(false)
  const [output, setOutput] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!isOpen) return
    let active = true
    let timeout: number | null = null
    let requestPending = false
    const poll = () => {
      if (requestPending) return
      requestPending = true
      const url = `/api/sessions/${encodeURIComponent(sessionId)}/background-tasks/${encodeURIComponent(task.id)}/output`
      void requestText(url)
        .match(
          (nextOutput) => {
            if (!active) return
            setOutput(nextOutput)
            setError(null)
          },
          (requestError) => {
            if (!active) return
            setError(requestError.message)
          },
        )
        .then(() => {
          requestPending = false
          if (active) timeout = window.setTimeout(poll, 1_000)
        })
    }
    poll()
    return () => {
      active = false
      if (timeout !== null) window.clearTimeout(timeout)
    }
  }, [isOpen, sessionId, task.id])

  return (
    <div className="running-task-content">
      <div className="running-task-heading">
        <span
          className="running-task-description running-task-description--command"
          title={task.description}
        >
          {task.description}
        </span>
        <button
          aria-controls={outputId}
          aria-expanded={isOpen}
          className="running-task-output-button"
          onClick={() => {
            setIsOpen((current) => !current)
          }}
          type="button"
        >
          Output
        </button>
      </div>
      <span className="running-task-elapsed">
        {formatElapsedTime(task.elapsedSeconds)}
      </span>
      {isOpen ? (
        <BackgroundTaskOutput id={outputId} output={output} error={error} />
      ) : null}
    </div>
  )
}
