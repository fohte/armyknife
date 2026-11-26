import {
  type AsyncAction,
  formatAsyncActionTime,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'

function renderConversationAsyncActionItem({
  action,
  feedback,
}: {
  action: AsyncAction
  feedback: NonNullable<ReturnType<typeof useOptionalAsyncActionFeedback>>
}) {
  return (
    <AsyncActionItem
      action={action}
      form="line"
      key={action.id}
      {...(action.kind === 'command'
        ? { time: formatAsyncActionTime(action.createdAt) }
        : {})}
      onCloseKeepWorktree={() => {
        feedback.closeWithWorktree(action.id)
      }}
      onRetry={() => {
        feedback.retry(action.id)
      }}
    />
  )
}

export function renderConversationAsyncActionItems({
  feedback,
  sessionId,
}: {
  feedback: ReturnType<typeof useOptionalAsyncActionFeedback>
  sessionId: string
}) {
  return feedback?.actions
    .filter(
      (action) =>
        action.sessionId === sessionId && action.timelineIndex === undefined,
    )
    .map((action) => renderConversationAsyncActionItem({ action, feedback }))
}
