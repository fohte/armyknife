import '#components/conversation-status-band.css'

import { Button } from '@fohte/ui/button'
import { Hourglass } from 'lucide-react'
import { useId, useState } from 'react'

import { useBackgroundTaskElapsedSeconds } from '#components/background-task-elapsed-context'
import type { ConversationStatusBandState } from '#components/conversation-status-band-state'
import { formatElapsedTime } from '#components/format-elapsed-time'
import { useSessionElapsedStartedAt } from '#components/session-elapsed-context'
import type { BackgroundTask, ToolOperation } from '#types'
import { useLiveElapsedOffsetSeconds } from '#use-live-elapsed-offset-seconds'

type ConversationStatusBandProps = {
  state: ConversationStatusBandState | null
  elapsedSeconds: number
  activeTool: ToolOperation | undefined
  backgroundTasks: readonly BackgroundTask[]
  isMobile?: boolean
  defaultExpanded?: boolean
  defaultBackgroundTasksExpanded?: boolean
  onReviewInCrit?: () => void
  onOpenDrafts?: () => void
  onReturnToCard?: () => void
}

const statusLabels: Record<ConversationStatusBandState['kind'], string> = {
  working: 'Working',
  compacting: 'Compacting…',
  'background-tasks': 'Background tasks',
  'crit-review': 'Waiting for your review',
  'draft-review': 'Waiting for your review',
  question: 'Question',
  permission: 'Permission',
  waiting: 'Waiting',
}

function WorkingTitle({ elapsedSeconds }: { elapsedSeconds: number }) {
  const startedAt = useSessionElapsedStartedAt()
  if (startedAt === null) {
    return <>Working · {formatElapsedTime(elapsedSeconds)}</>
  }
  return (
    <LiveWorkingTitle elapsedSeconds={elapsedSeconds} startedAt={startedAt} />
  )
}

function LiveWorkingTitle({
  elapsedSeconds,
  startedAt,
}: {
  elapsedSeconds: number
  startedAt: number
}) {
  const elapsedOffsetSeconds = useLiveElapsedOffsetSeconds(startedAt)
  return (
    <>Working · {formatElapsedTime(elapsedSeconds + elapsedOffsetSeconds)}</>
  )
}

export function ConversationStatusBand({
  state,
  elapsedSeconds,
  activeTool,
  backgroundTasks,
  isMobile = false,
  defaultExpanded = false,
  defaultBackgroundTasksExpanded = false,
  onReviewInCrit,
  onOpenDrafts,
  onReturnToCard,
}: ConversationStatusBandProps) {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded)
  const detailsId = useId()
  if (state === null) return null

  const isReview = state.kind === 'crit-review' || state.kind === 'draft-review'
  const isDraftReview = state.kind === 'draft-review'
  const isWarning =
    isReview || state.kind === 'question' || state.kind === 'permission'
  const title =
    state.kind === 'working' ? (
      <WorkingTitle elapsedSeconds={elapsedSeconds} />
    ) : state.kind === 'background-tasks' ? (
      `Background tasks (${String(backgroundTasks.length)})`
    ) : (
      statusLabels[state.kind]
    )
  const backgroundTask = backgroundTasks[0]
  const backgroundTaskDetails = backgroundTasks.map((task) => ({
    key: `task-${task.id}`,
    content: <BackgroundTaskDetail task={task} />,
  }))
  const toolName = (tool: string) =>
    tool.toLowerCase() === 'exec' ? 'Bash' : tool
  const activeToolDetail =
    activeTool === undefined
      ? null
      : activeTool.target === undefined || activeTool.target === ''
        ? toolName(activeTool.tool)
        : `${toolName(activeTool.tool)}  ${activeTool.target}`
  const detail =
    state.kind === 'working' && activeToolDetail !== null ? (
      activeToolDetail
    ) : state.kind === 'compacting' ? (
      '古い会話を要約しています'
    ) : state.kind === 'background-tasks' && backgroundTask !== undefined ? (
      <BackgroundTaskDetail task={backgroundTask} />
    ) : (
      state.detail
    )
  const inlineDetail =
    state.kind === 'waiting' && detail == null && backgroundTasks.length > 0
      ? `Background tasks (${String(backgroundTasks.length)})`
      : detail
  const hasInlineDetail = inlineDetail != null && inlineDetail !== ''
  const isCardReturn = state.kind === 'question' || state.kind === 'permission'

  if (isReview) {
    return (
      <section
        className={`conversation-status-band conversation-status-band--review is-waiting is-warning${isMobile ? ' is-mobile' : ''}`}
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
            <BackgroundTaskDisclosure
              tasks={backgroundTasks}
              defaultOpen={defaultBackgroundTasksExpanded}
            />
          ) : null}
        </div>
        <Button
          className="conversation-status-band-review"
          type="button"
          variant="secondary"
          onClick={isDraftReview ? onOpenDrafts : onReviewInCrit}
        >
          {isDraftReview ? 'Open drafts' : 'Review in crit'}{' '}
          <span aria-hidden="true">→</span>
        </Button>
      </section>
    )
  }

  const expandedDetails =
    state.kind === 'background-tasks' && backgroundTaskDetails.length > 0
      ? backgroundTaskDetails
      : [
          ...(detail == null || detail === ''
            ? []
            : [{ key: 'state-detail', content: detail }]),
          ...backgroundTaskDetails,
        ]
  const statusToggle = (
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
      {state.kind === 'waiting' ? (
        <Hourglass
          className="conversation-status-band-hourglass"
          aria-hidden="true"
          size={14}
        />
      ) : (
        <span className="conversation-status-band-dot" aria-hidden="true" />
      )}
      <strong className="conversation-status-band-label">{title}</strong>
      {hasInlineDetail ? (
        <span className="conversation-status-band-separator" aria-hidden="true">
          ·
        </span>
      ) : null}
      <span
        className={`conversation-status-band-inline-detail${state.kind === 'working' || state.kind === 'permission' ? ' is-monospace' : ''}`}
        aria-hidden={hasInlineDetail ? undefined : true}
      >
        {hasInlineDetail ? inlineDetail : null}
      </span>
      <span className="conversation-status-band-chevron" aria-hidden="true">
        {isCardReturn ? '↑' : isExpanded ? '▾' : '›'}
      </span>
    </Button>
  )

  return (
    <section
      className={`conversation-status-band conversation-status-band--compact conversation-status-band--${state.kind}${isWarning ? ' is-warning' : ''}${isMobile ? ' is-mobile' : ''}`}
      aria-label="Conversation status"
      role="status"
    >
      {isCardReturn && backgroundTasks.length > 0 ? (
        <div className="conversation-status-band-compact-main">
          {statusToggle}
          <BackgroundTaskDisclosure
            tasks={backgroundTasks}
            defaultOpen={defaultBackgroundTasksExpanded}
          />
        </div>
      ) : (
        statusToggle
      )}
      {isExpanded && !isCardReturn ? (
        <div
          className="conversation-status-band-expanded"
          id={detailsId}
          aria-label="Full status details"
        >
          {expandedDetails.map(({ key, content }) => (
            <p key={key}>{content}</p>
          ))}
        </div>
      ) : null}
    </section>
  )
}

function BackgroundTaskDetail({ task }: { task: BackgroundTask }) {
  return (
    <>
      {task.description} ·{' '}
      <BackgroundTaskElapsedTime elapsedSeconds={task.elapsedSeconds} />
    </>
  )
}

function BackgroundTaskElapsedTime({
  elapsedSeconds,
}: {
  elapsedSeconds: number
}) {
  return formatElapsedTime(useBackgroundTaskElapsedSeconds(elapsedSeconds))
}

function BackgroundTaskDisclosure({
  tasks,
  defaultOpen = false,
}: {
  tasks: readonly BackgroundTask[]
  defaultOpen?: boolean
}) {
  return (
    <details className="conversation-status-band-tasks" open={defaultOpen}>
      <summary>
        Background tasks ({String(tasks.length)}){' '}
        <span aria-hidden="true">▾</span>
      </summary>
      <BackgroundTaskList tasks={tasks} />
    </details>
  )
}

function BackgroundTaskList({ tasks }: { tasks: readonly BackgroundTask[] }) {
  return (
    <ul>
      {tasks.map((task) => (
        <li key={task.id}>
          <span>{task.description}</span>
          <span>
            <BackgroundTaskElapsedTime elapsedSeconds={task.elapsedSeconds} />
          </span>
        </li>
      ))}
    </ul>
  )
}
