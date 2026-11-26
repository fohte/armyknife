import '#components/running-tasks-panel.css'

import { useId } from 'react'

import { useBackgroundTaskElapsedSeconds } from '#components/background-task-elapsed-context'
import { formatElapsedTime } from '#components/format-elapsed-time'
import { BackgroundTaskOutput } from '#components/running-task-output'
import type { BackgroundTask } from '#types'

type BackgroundTaskRowViewProps = {
  task: BackgroundTask
  isOpen: boolean
  output: string | null
  error: string | null
  onToggle: () => void
}

export function BackgroundTaskRowView({
  task,
  isOpen,
  output,
  error,
  onToggle,
}: BackgroundTaskRowViewProps) {
  const outputId = useId()

  return (
    <li className="running-task">
      <span className="running-task-icon" aria-hidden="true">
        ▶
      </span>
      <div className="running-task-content running-task-content--background">
        <div className="running-task-heading">
          <span
            className="running-task-description running-task-description--command"
            title={task.description}
          >
            {task.description}
          </span>
        </div>
        <div className="running-task-controls">
          <span className="running-task-elapsed">
            <BackgroundTaskElapsedTime elapsedSeconds={task.elapsedSeconds} />
          </span>
          <button
            aria-controls={outputId}
            aria-expanded={isOpen}
            className="running-task-output-button"
            onClick={onToggle}
            type="button"
          >
            Output
          </button>
        </div>
        {isOpen ? (
          <BackgroundTaskOutput id={outputId} output={output} error={error} />
        ) : null}
      </div>
    </li>
  )
}

function BackgroundTaskElapsedTime({
  elapsedSeconds,
}: {
  elapsedSeconds: number
}) {
  return formatElapsedTime(useBackgroundTaskElapsedSeconds(elapsedSeconds))
}
