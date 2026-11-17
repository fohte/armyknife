import '#components/conversation-status-band.css'

import { Button } from '@fohte/ui/button'

import type { ConversationStatusBandState } from '#components/conversation-status-band-state'
import { formatElapsedTime } from '#components/format-elapsed-time'
import type { BackgroundTask, ToolOperation } from '#types'

type ConversationStatusBandProps = {
  state: ConversationStatusBandState | null
  elapsedSeconds: number
  activeTool: ToolOperation | undefined
  backgroundTasks: readonly BackgroundTask[]
  isMobile?: boolean
  onReviewInCrit?: () => void
}

const statusLabels: Record<ConversationStatusBandState['kind'], string> = {
  working: 'Working',
  compacting: 'Compacting conversation…',
  'background-tasks': 'Waiting for background tasks',
  'crit-review': 'Waiting for your review',
  question: 'Waiting for your answer',
  permission: 'Waiting for permission',
  waiting: 'Waiting',
}

export function ConversationStatusBand({
  state,
  elapsedSeconds,
  activeTool,
  backgroundTasks,
  isMobile = false,
  onReviewInCrit,
}: ConversationStatusBandProps) {
  if (state === null) return null

  const isReview = state.kind === 'crit-review'
  const isWarning =
    state.kind === 'crit-review' ||
    state.kind === 'question' ||
    state.kind === 'permission'
  const title =
    state.kind === 'working'
      ? `Working · ${formatElapsedTime(elapsedSeconds)}`
      : statusLabels[state.kind]
  const backgroundTask = backgroundTasks[0]
  const toolName = (tool: string) =>
    tool.toLowerCase() === 'exec' ? 'Bash' : tool
  const activeToolDetail =
    activeTool === undefined
      ? null
      : activeTool.target === undefined || activeTool.target === ''
        ? toolName(activeTool.tool)
        : `${toolName(activeTool.tool)}  ${activeTool.target}`
  const detail =
    state.kind === 'working' && activeToolDetail !== null
      ? activeToolDetail
      : state.kind === 'compacting'
        ? '古い会話を要約しています'
        : state.kind === 'background-tasks' && backgroundTask !== undefined
          ? `${backgroundTask.description} · ${formatElapsedTime(backgroundTask.elapsedSeconds)}`
          : state.detail

  return (
    <section
      className={`conversation-status-band conversation-status-band--${state.kind}${state.kind !== 'working' && state.kind !== 'compacting' ? ' is-waiting' : ''}${isWarning ? ' is-warning' : ''}${isMobile ? ' is-mobile' : ''}`}
      aria-label="Conversation status"
      role="status"
    >
      <div className="conversation-status-band-main">
        <div className="conversation-status-band-copy">
          <div className="conversation-status-band-title">
            <span className="conversation-status-band-dot" aria-hidden="true" />
            <strong>{title}</strong>
          </div>
          {detail == null || detail === '' ? null : (
            <p
              className={`conversation-status-band-detail${state.kind === 'working' || state.kind === 'permission' ? ' is-monospace' : ''}`}
            >
              {detail}
            </p>
          )}
        </div>
        {backgroundTasks.length > 0 ? (
          <details className="conversation-status-band-tasks">
            <summary>
              Background tasks ({String(backgroundTasks.length)}){' '}
              <span aria-hidden="true">▾</span>
            </summary>
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
      {isReview ? (
        <Button
          className="conversation-status-band-review"
          type="button"
          variant="secondary"
          onClick={onReviewInCrit}
        >
          Review in crit <span aria-hidden="true">→</span>
        </Button>
      ) : null}
    </section>
  )
}
