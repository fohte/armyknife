import '#components/running-tasks-panel.css'

import type { ReactNode } from 'react'

import { formatElapsedTime } from '#components/format-elapsed-time'
import type { BackgroundTask } from '#types'

type RunningTasksPanelViewProps = {
  subagents: readonly BackgroundTask[]
  backgroundTaskRows: readonly ReactNode[]
}

export function RunningTasksPanelView({
  subagents,
  backgroundTaskRows,
}: RunningTasksPanelViewProps) {
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
      {backgroundTaskRows.length > 0 ? (
        <section className="running-tasks-group" aria-label="Background tasks">
          <h2>BACKGROUND TASKS</h2>
          <ul>{backgroundTaskRows}</ul>
        </section>
      ) : null}
    </div>
  )
}
