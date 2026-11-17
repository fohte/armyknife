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
  compacting: 'Summarizing conversation',
  'background-tasks': 'Background tasks',
  'crit-review': 'Review needed',
  question: 'Question',
  permission: 'Permission needed',
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
  const title =
    state.kind === 'working'
      ? `Working · ${formatElapsedTime(elapsedSeconds)}`
      : statusLabels[state.kind]
  const detail =
    state.kind === 'working' && activeTool !== undefined
      ? [activeTool.tool, activeTool.target].filter(Boolean).join(' · ')
      : state.detail

  return (
    <section
      className={`conversation-status-band conversation-status-band--${state.kind}${state.kind !== 'working' && state.kind !== 'compacting' ? ' is-waiting' : ''}${isMobile ? ' is-mobile' : ''}`}
      aria-label="Conversation status"
      role="status"
    >
      <div className="conversation-status-band-copy">
        <div className="conversation-status-band-title">
          <span className="conversation-status-band-dot" aria-hidden="true" />
          <strong>{title}</strong>
        </div>
        {detail == null || detail === '' ? null : (
          <p className="conversation-status-band-detail">{detail}</p>
        )}
      </div>
      {isReview ? (
        <Button
          className="conversation-status-band-review"
          type="button"
          variant="secondary"
          onClick={onReviewInCrit}
        >
          Review in crit
        </Button>
      ) : null}
      {backgroundTasks.length > 0 ? (
        <details className="conversation-status-band-tasks">
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
    </section>
  )
}
