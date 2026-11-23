import { getPendingCritReviewUrl } from '#components/conversation-crit-url'
import type {
  EditorReviewDocumentState,
  ReviewDocumentSummary,
} from '#components/review-document-summary'
import type { BackgroundTask, ChatItem, Session } from '#types'

export type ConversationStatusBandState = {
  kind:
    | 'working'
    | 'compacting'
    | 'background-tasks'
    | 'crit-review'
    | 'draft-review'
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

function critReviewDetail(session: Session) {
  const pullRequests =
    session.pullRequests !== undefined && session.pullRequests.length > 0
      ? session.pullRequests
      : session.pullRequest === null
        ? []
        : [session.pullRequest]
  const pullRequest = pullRequests[0]
  return pullRequest === undefined
    ? 'crit'
    : `crit · PR #${String(pullRequest.number)}`
}

function editorDocumentDetail(document: ReviewDocumentSummary | null) {
  if (document === null) return 'Draft'
  switch (document.kind) {
    case 'pr_draft':
      return `PR draft${document.target.branch === undefined ? '' : ` · ${document.target.branch}`}`
    case 'issue_comment':
      return document.target.issueNumber === undefined
        ? 'Issue comment'
        : `Issue comment #${String(document.target.issueNumber)}`
    case 'issue_body':
      return document.target.issueNumber === undefined
        ? 'Issue body'
        : `Issue body #${String(document.target.issueNumber)}`
    case 'review_replies':
      return document.target.pullRequestNumber === undefined
        ? 'Review replies'
        : `Review replies on PR #${String(document.target.pullRequestNumber)}`
    case 'draft':
      return document.target.title === undefined
        ? 'Draft'
        : `Draft: ${document.target.title}`
  }
}

export function getConversationStatusBandState({
  session,
  messages,
  backgroundTasks,
  editorReviewDocument = null,
}: {
  session: Session
  messages: readonly ChatItem[]
  backgroundTasks: readonly BackgroundTask[]
  editorReviewDocument?: EditorReviewDocumentState | null
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
        detail: `${permission.tool.toLowerCase() === 'exec' ? 'Bash' : permission.tool}  ${permission.command ?? permission.description}`,
      }
    }
  }
  if (session.reviewDocumentId != null) {
    const documentCount = Math.max(1, session.reviewDocumentCount ?? 1)
    const draftLabel = `${String(documentCount)} ${documentCount === 1 ? 'draft' : 'drafts'}`
    const editorDetail =
      editorReviewDocument?.status === 'unavailable'
        ? 'Draft details are unavailable'
        : `${editorDocumentDetail(
            editorReviewDocument?.status === 'ready'
              ? editorReviewDocument.summary
              : null,
          )} is in the editor`
    const critDetail =
      isWaitingForUser && getPendingCritReviewUrl(session) !== null
        ? ` · ${critReviewDetail(session)}`
        : ''
    return {
      kind: 'draft-review',
      detail: `${draftLabel} · ${editorDetail}${critDetail}`,
    }
  }
  if (isWorking) return { kind: 'working', detail: null }
  if (isWaitingForUser) {
    if (getPendingCritReviewUrl(session) !== null) {
      return {
        kind: 'crit-review',
        detail: critReviewDetail(session),
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
