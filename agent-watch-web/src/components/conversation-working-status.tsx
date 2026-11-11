import '#components/conversation-working-status.css'

import { formatElapsedTime } from '#components/format-elapsed-time'
import type { BackgroundTask, ToolOperation } from '#types'

type ConversationWorkingStatusProps = {
  showWorking?: boolean
  elapsedSeconds: number
  activeTool: ToolOperation | undefined
  backgroundTasks?: readonly BackgroundTask[]
  showBackgroundTasks?: boolean
  isCompacting?: boolean
}

export function ConversationWorkingStatus({
  showWorking = true,
  elapsedSeconds,
  activeTool,
  backgroundTasks = [],
  showBackgroundTasks = true,
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
      {showBackgroundTasks && backgroundTasks.length > 0 ? (
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
