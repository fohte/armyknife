import '#components/conversation-working-status.css'

import type { BackgroundTask, ToolOperation } from '#types'

type ConversationWorkingStatusProps = {
  showWorking?: boolean
  elapsedSeconds: number
  activeTool: ToolOperation | undefined
  backgroundTasks?: readonly BackgroundTask[]
  isCompacting?: boolean
}

function formatElapsedTime(totalSeconds: number) {
  if (totalSeconds < 60) return `${String(totalSeconds)}s`
  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) {
    const seconds = totalSeconds % 60
    return seconds === 0
      ? `${String(minutes)}m`
      : `${String(minutes)}m ${String(seconds)}s`
  }
  const hours = Math.floor(minutes / 60)
  return `${String(hours)}h ${String(minutes % 60)}m`
}

export function ConversationWorkingStatus({
  showWorking = true,
  elapsedSeconds,
  activeTool,
  backgroundTasks = [],
  isCompacting = false,
}: ConversationWorkingStatusProps) {
  return (
    <div className="conversation-working-status" role="status">
      {showWorking || isCompacting ? (
        <>
          <span className="conversation-working-dot" aria-hidden="true" />
          <span>
            {isCompacting
              ? 'Compacting conversation…'
              : `Working · ${formatElapsedTime(elapsedSeconds)}`}
          </span>
          {!isCompacting && activeTool != null ? (
            <span className="conversation-working-tool">
              {activeTool.tool}
              {activeTool.target != null && activeTool.target !== '' ? (
                <span className="conversation-working-target">
                  {activeTool.target}
                </span>
              ) : null}
            </span>
          ) : null}
        </>
      ) : null}
      {backgroundTasks.length > 0 ? (
        <details className="conversation-background-tasks">
          <summary>Background tasks ({String(backgroundTasks.length)})</summary>
          <ul>
            {backgroundTasks.map((task) => (
              <li key={task.id}>
                <span>{task.description}</span>
                <span>{formatElapsedTime(task.elapsedSeconds)}</span>
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </div>
  )
}
