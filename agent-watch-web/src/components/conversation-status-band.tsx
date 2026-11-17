import '#components/conversation-status-band.css'

import { Button } from '@fohte/ui/button'
import { useId, useState } from 'react'

import type { ConversationStatusBandState } from '#components/conversation-status-band-state'
import { formatElapsedTime } from '#components/format-elapsed-time'
import type { BackgroundTask, ToolOperation } from '#types'

type ConversationStatusBandProps = {
  state: ConversationStatusBandState | null
  elapsedSeconds: number
  activeTool: ToolOperation | undefined
  backgroundTasks: readonly BackgroundTask[]
  isMobile?: boolean
  defaultExpanded?: boolean
  onReviewInCrit?: () => void
  onReturnToCard?: () => void
}

const statusLabels: Record<ConversationStatusBandState['kind'], string> = {
  working: 'Working',
  compacting: 'Compacting…',
  'background-tasks': 'Background tasks',
  'crit-review': 'Waiting for your review',
  question: 'Question',
  permission: 'Permission',
  waiting: 'Waiting',
}

export function ConversationStatusBand({
  state,
  elapsedSeconds,
  activeTool,
  backgroundTasks,
  isMobile = false,
  defaultExpanded = false,
  onReviewInCrit,
  onReturnToCard,
}: ConversationStatusBandProps) {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded)
  const detailsId = useId()
  if (state === null) return null

  const isReview = state.kind === 'crit-review'
  const isWarning =
    state.kind === 'crit-review' ||
    state.kind === 'question' ||
    state.kind === 'permission'
  const title =
    state.kind === 'working'
      ? `Working · ${formatElapsedTime(elapsedSeconds)}`
      : state.kind === 'background-tasks'
        ? `Background tasks (${String(backgroundTasks.length)})`
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
  const inlineDetail =
    state.kind === 'waiting' && detail == null && backgroundTasks.length > 0
      ? `Background tasks (${String(backgroundTasks.length)})`
      : detail
  const isCardReturn = state.kind === 'question' || state.kind === 'permission'

  if (isReview) {
    return (
      <section
        className={`conversation-status-band conversation-status-band--crit-review is-waiting is-warning${isMobile ? ' is-mobile' : ''}`}
        aria-label="Conversation status"
        role="status"
      >
        <div className="conversation-status-band-main">
          <div className="conversation-status-band-copy">
            <div className="conversation-status-band-title">
              <span
                className="conversation-status-band-dot"
                aria-hidden="true"
              />
              <strong>{title}</strong>
            </div>
            {detail == null || detail === '' ? null : (
              <p className="conversation-status-band-detail">{detail}</p>
            )}
          </div>
          {backgroundTasks.length > 0 ? (
            <details className="conversation-status-band-tasks">
              <summary>
                Background tasks ({String(backgroundTasks.length)}){' '}
                <span aria-hidden="true">▾</span>
              </summary>
              <BackgroundTaskList tasks={backgroundTasks} />
            </details>
          ) : null}
        </div>
        <Button
          className="conversation-status-band-review"
          type="button"
          variant="secondary"
          onClick={onReviewInCrit}
        >
          Review in crit <span aria-hidden="true">→</span>
        </Button>
      </section>
    )
  }

  const backgroundTaskDetails = backgroundTasks.map(
    (task) => `${task.description} · ${formatElapsedTime(task.elapsedSeconds)}`,
  )
  const expandedDetails =
    state.kind === 'background-tasks' && backgroundTaskDetails.length > 0
      ? backgroundTaskDetails
      : [
          ...(detail == null || detail === '' ? [] : [detail]),
          ...(state.kind === 'waiting' ? backgroundTaskDetails : []),
        ]

  return (
    <section
      className={`conversation-status-band conversation-status-band--compact conversation-status-band--${state.kind}${isWarning ? ' is-warning' : ''}${isMobile ? ' is-mobile' : ''}`}
      aria-label="Conversation status"
      role="status"
    >
      <Button
        className="conversation-status-band-toggle"
        type="button"
        variant="plain"
        aria-expanded={isCardReturn ? undefined : isExpanded}
        aria-controls={isCardReturn ? undefined : detailsId}
        onClick={
          isCardReturn
            ? () => onReturnToCard?.()
            : () => {
                setIsExpanded((expanded) => !expanded)
              }
        }
      >
        <span className="conversation-status-band-dot" aria-hidden="true" />
        <strong className="conversation-status-band-label">{title}</strong>
        {inlineDetail == null || inlineDetail === '' ? null : (
          <>
            <span
              className="conversation-status-band-separator"
              aria-hidden="true"
            >
              ·
            </span>
            <span
              className={`conversation-status-band-inline-detail${state.kind === 'working' || state.kind === 'permission' ? ' is-monospace' : ''}`}
            >
              {inlineDetail}
            </span>
          </>
        )}
        <span className="conversation-status-band-chevron" aria-hidden="true">
          {isCardReturn ? '↑' : isExpanded ? '▾' : '›'}
        </span>
      </Button>
      {isExpanded && !isCardReturn ? (
        <div
          className="conversation-status-band-expanded"
          id={detailsId}
          aria-label="Full status details"
        >
          {expandedDetails.map((item) => (
            <p key={item}>{item}</p>
          ))}
        </div>
      ) : null}
    </section>
  )
}

function BackgroundTaskList({ tasks }: { tasks: readonly BackgroundTask[] }) {
  return (
    <ul>
      {tasks.map((task) => (
        <li key={task.id}>
          <span>{task.description}</span>
          <span>{formatElapsedTime(task.elapsedSeconds)}</span>
        </li>
      ))}
    </ul>
  )
}
