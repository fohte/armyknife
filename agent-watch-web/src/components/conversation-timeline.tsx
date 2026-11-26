import '#components/conversation-pane.css'
import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import type { ReactNode, Ref } from 'react'

import { type AsyncAction, formatAsyncActionTime } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'
import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import { MarkdownCommandContextProvider } from '#components/markdown-command-context'

type ConversationTimelineMessage = {
  element: ReactNode
  index?: number
}

type ConversationTimelineProps = {
  actions: readonly AsyncAction[]
  hasActiveTool: boolean
  hasOlderMessages: boolean
  loading: boolean
  loadingOlder: boolean
  messageCount: number
  messages: readonly ConversationTimelineMessage[]
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
  onLoadOlder: () => void
  onRetry?: (actionId: number) => void
  onScroll: () => void
  scrollRef: Ref<HTMLDivElement>
  sessionId: string
  sessionTitle: string
  start: number
  trailingActions?: ReactNode
}

export function ConversationTimeline({
  actions,
  hasActiveTool,
  hasOlderMessages,
  loading,
  loadingOlder,
  messageCount,
  messages,
  onCommandExecuted,
  onCommandStarted,
  onLoadOlder,
  onRetry,
  onScroll,
  scrollRef,
  sessionId,
  sessionTitle,
  start,
  trailingActions,
}: ConversationTimelineProps) {
  const positionedActions = actions
    .filter(
      (action) =>
        action.sessionId === sessionId &&
        action.timelineIndex !== undefined &&
        action.timelineIndex >= start,
    )
    .toSorted(
      (left, right) =>
        (left.timelineIndex ?? 0) - (right.timelineIndex ?? 0) ||
        left.createdAt - right.createdAt ||
        left.id - right.id,
    )
  const elements: ReactNode[] = []
  const renderPositionedAction = (action: AsyncAction) => (
    <AsyncActionItem
      action={action}
      form="line"
      key={action.id}
      {...(action.kind === 'command'
        ? { time: formatAsyncActionTime(action.createdAt) }
        : action.completedAt === undefined
          ? {}
          : { time: formatAsyncActionTime(action.completedAt) })}
      {...(onRetry === undefined
        ? {}
        : {
            onRetry: () => {
              onRetry(action.id)
            },
          })}
    />
  )
  let nextAction = 0
  for (const message of messages) {
    while (
      nextAction < positionedActions.length &&
      message.index !== undefined &&
      (positionedActions[nextAction]?.timelineIndex ?? Infinity) <=
        message.index
    ) {
      const action = positionedActions[nextAction]
      if (action !== undefined) elements.push(renderPositionedAction(action))
      nextAction += 1
    }
    elements.push(message.element)
  }
  while (nextAction < positionedActions.length) {
    const action = positionedActions[nextAction]
    if (action !== undefined) elements.push(renderPositionedAction(action))
    nextAction += 1
  }

  return (
    <div
      ref={scrollRef}
      className="conversation-scroll"
      aria-live="polite"
      onScroll={onScroll}
    >
      {hasOlderMessages ? (
        <Button
          className="conversation-load-older"
          type="button"
          variant="plain"
          disabled={loadingOlder}
          onClick={onLoadOlder}
        >
          {loadingOlder ? 'Loading older messages…' : 'Load older messages'}
        </Button>
      ) : null}
      {loading && messageCount === 0 ? (
        <p className="conversation-status">Loading conversation…</p>
      ) : null}
      {!loading && messageCount === 0 && !hasActiveTool ? (
        <p className="conversation-status">No messages yet.</p>
      ) : null}
      <MarkdownCommandContextProvider
        sessionId={sessionId}
        sessionTitle={sessionTitle}
        {...(onCommandExecuted === undefined ? {} : { onCommandExecuted })}
        {...(onCommandStarted === undefined ? {} : { onCommandStarted })}
      >
        {elements}
      </MarkdownCommandContextProvider>
      {trailingActions}
    </div>
  )
}
