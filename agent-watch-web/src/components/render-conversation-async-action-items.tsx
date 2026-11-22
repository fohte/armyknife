import {
  formatAsyncActionTime,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'

export function renderConversationAsyncActionItems({
  feedback,
  sessionId,
}: {
  feedback: ReturnType<typeof useOptionalAsyncActionFeedback>
  sessionId: string
}) {
  return feedback?.actions
    .filter((action) => action.sessionId === sessionId)
    .map((action) => (
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
    ))
}
