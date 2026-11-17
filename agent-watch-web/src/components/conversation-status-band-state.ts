import { getPendingCritReviewUrl } from '#components/conversation-crit-url'
import type { BackgroundTask, ChatItem, Session } from '#types'

export type ConversationStatusBandState = {
  kind:
    | 'working'
    | 'compacting'
    | 'background-tasks'
    | 'crit-review'
    | 'question'
    | 'permission'
    | 'waiting'
  detail: string | null
}

function pendingQuestion(messages: readonly ChatItem[]) {
  return [...messages]
    .reverse()
    .find(
      (message) =>
        message.type === 'question' &&
        message.cancelled !== true &&
        message.questions.some((question) => question.answer === null),
    )
}

function pendingPermission(messages: readonly ChatItem[]) {
  return [...messages]
    .reverse()
    .find((message) => message.type === 'permission')
}

export function getConversationStatusBandState({
  session,
  messages,
  backgroundTasks,
}: {
  session: Session
  messages: readonly ChatItem[]
  backgroundTasks: readonly BackgroundTask[]
}): ConversationStatusBandState | null {
  const isCompacting = session.isCompacting === true
  const isWaitingForBackgroundTasks = session.waitingForBackgroundTasks === true
  const isWaitingForUser = session.category === 'needs_action'
  const isWaitingForExternalWork =
    session.category === 'in_progress' && session.activityStatus === 'waiting'
  const isWorking =
    session.category === 'in_progress' &&
    !isWaitingForBackgroundTasks &&
    (session.activityStatus === 'working' || session.activityStatus == null)

  if (isCompacting) {
    return { kind: 'compacting', detail: null }
  }
  if (isWorking) return { kind: 'working', detail: null }
  if (isWaitingForUser) {
    const question = pendingQuestion(messages)
    if (question?.type === 'question') {
      return {
        kind: 'question',
        detail:
          question.questions.find((prompt) => prompt.answer === null)
            ?.question ?? null,
      }
    }
    const permission = pendingPermission(messages)
    if (permission?.type === 'permission') {
      return {
        kind: 'permission',
        detail: permission.command ?? permission.description,
      }
    }
    if (getPendingCritReviewUrl(session) !== null) {
      return {
        kind: 'crit-review',
        detail: session.task?.title ?? session.title,
      }
    }
    return {
      kind: 'waiting',
      detail: session.waitingReason ?? session.preview,
    }
  }

  if (isWaitingForBackgroundTasks) {
    return {
      kind: 'background-tasks',
      detail:
        backgroundTasks[0]?.description ?? session.task?.title ?? session.title,
    }
  }

  if (isWaitingForExternalWork) {
    return { kind: 'waiting', detail: session.waitingReason ?? session.preview }
  }

  return null
}
