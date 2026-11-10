import type { ToolOperation } from '#types'

type ConversationWorkingStatusProps = {
  elapsedSeconds: number
  activeTool: ToolOperation | undefined
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
  elapsedSeconds,
  activeTool,
}: ConversationWorkingStatusProps) {
  return (
    <div className="conversation-working-status" role="status">
      <span className="conversation-working-dot" aria-hidden="true" />
      <span>Working · {formatElapsedTime(elapsedSeconds)}</span>
      {activeTool != null ? (
        <span className="conversation-working-tool">
          {activeTool.tool}
          {activeTool.target != null && activeTool.target !== '' ? (
            <span className="conversation-working-target">
              {activeTool.target}
            </span>
          ) : null}
        </span>
      ) : null}
    </div>
  )
}
