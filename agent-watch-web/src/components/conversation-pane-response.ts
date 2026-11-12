import { isPullRequest, isSessionTask } from '#session-record-validation'
import type {
  BackgroundTask,
  ChatItem,
  ConversationResponse,
  SessionActions,
  SessionPullRequestListResponse,
  SessionTreeNode,
  SkillSuggestion,
} from '#types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function hasValidOptionalTimestamp(value: Record<string, unknown>): boolean {
  return (
    value['timestamp'] === undefined || typeof value['timestamp'] === 'string'
  )
}

function isUserContentPart(value: unknown): boolean {
  if (!isRecord(value) || typeof value['type'] !== 'string') return false
  if (value['type'] === 'text') return typeof value['text'] === 'string'
  if (value['type'] === 'file') return typeof value['name'] === 'string'
  if (value['type'] !== 'image') return false
  return (
    typeof value['source'] === 'string' &&
    (value['source'].startsWith('/api/images/') ||
      /^data:image\/(png|jpeg|gif|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(
        value['source'],
      )) &&
    typeof value['available'] === 'boolean'
  )
}

function isProposalOption(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value['label'] === 'string' &&
    (value['description'] === null ||
      typeof value['description'] === 'string') &&
    Array.isArray(value['pros']) &&
    value['pros'].every((pro) => typeof pro === 'string') &&
    Array.isArray(value['cons']) &&
    value['cons'].every((con) => typeof con === 'string') &&
    typeof value['recommended'] === 'boolean'
  )
}

function isProposalAnswer(value: unknown): boolean {
  return (
    isRecord(value) &&
    (value['choice'] === null ||
      (typeof value['choice'] === 'number' &&
        Number.isInteger(value['choice']) &&
        value['choice'] >= 0)) &&
    (value['comment'] === null || typeof value['comment'] === 'string')
  )
}

function isChatItem(value: unknown): value is ChatItem {
  if (!isRecord(value) || typeof value['type'] !== 'string') return false
  switch (value['type']) {
    case 'assistant':
      return typeof value['text'] === 'string'
    case 'user':
      return (
        typeof value['text'] === 'string' && hasValidOptionalTimestamp(value)
      )
    case 'queued_user':
    case 'user_with_content':
      return (
        Array.isArray(value['content']) &&
        value['content'].every(isUserContentPart) &&
        (value['from'] === undefined || typeof value['from'] === 'string') &&
        hasValidOptionalTimestamp(value)
      )
    case 'peer':
      return (
        (value['direction'] === 'from' || value['direction'] === 'to') &&
        typeof value['session_id'] === 'string' &&
        (value['session_title'] === undefined ||
          typeof value['session_title'] === 'string') &&
        Array.isArray(value['content']) &&
        value['content'].every(isUserContentPart) &&
        hasValidOptionalTimestamp(value)
      )
    case 'subagent_report':
      return (
        typeof value['agent_id'] === 'string' &&
        typeof value['agent_name'] === 'string' &&
        (value['description'] === undefined ||
          typeof value['description'] === 'string') &&
        typeof value['text'] === 'string'
      )
    case 'event':
      return (
        (value['kind'] === 'armyknife' ||
          value['kind'] === 'compact_boundary' ||
          value['kind'] === 'system' ||
          value['kind'] === 'hook') &&
        typeof value['text'] === 'string' &&
        typeof value['failed'] === 'boolean' &&
        hasValidOptionalTimestamp(value)
      )
    case 'tool_summary':
      return (
        typeof value['summary'] === 'string' &&
        (value['operations'] === undefined ||
          (Array.isArray(value['operations']) &&
            value['operations'].every(
              (operation) =>
                isRecord(operation) &&
                typeof operation['tool'] === 'string' &&
                (operation['target'] === undefined ||
                  typeof operation['target'] === 'string') &&
                (operation['failed'] === undefined ||
                  typeof operation['failed'] === 'boolean'),
            )))
      )
    case 'tool_in_progress':
      return (
        typeof value['tool'] === 'string' &&
        (value['target'] === undefined || typeof value['target'] === 'string')
      )
    case 'question':
      return (
        (value['requestId'] === undefined ||
          typeof value['requestId'] === 'string') &&
        Array.isArray(value['questions']) &&
        value['questions'].length > 0 &&
        value['questions'].every(
          (question) =>
            isRecord(question) &&
            typeof question['question'] === 'string' &&
            (question['header'] === undefined ||
              typeof question['header'] === 'string') &&
            Array.isArray(question['choices']) &&
            question['choices'].every(
              (choice) =>
                isRecord(choice) &&
                typeof choice['label'] === 'string' &&
                (choice['description'] === undefined ||
                  typeof choice['description'] === 'string'),
            ) &&
            typeof question['multi_select'] === 'boolean' &&
            (question['answer'] === null ||
              typeof question['answer'] === 'string'),
        )
      )
    case 'proposal':
      return (
        typeof value['seq'] === 'number' &&
        Number.isInteger(value['seq']) &&
        value['seq'] > 0 &&
        typeof value['title'] === 'string' &&
        (value['summary'] === null || typeof value['summary'] === 'string') &&
        Array.isArray(value['options']) &&
        value['options'].every(isProposalOption) &&
        (value['answer'] === null || isProposalAnswer(value['answer'])) &&
        typeof value['resolved'] === 'boolean'
      )
    case 'todo':
      return (
        typeof value['seq'] === 'number' &&
        Number.isInteger(value['seq']) &&
        value['seq'] > 0 &&
        typeof value['title'] === 'string' &&
        (value['detail'] === null || typeof value['detail'] === 'string') &&
        typeof value['completed'] === 'boolean' &&
        (value['comment'] === null || typeof value['comment'] === 'string')
      )
    case 'permission':
      return (
        (value['requestId'] === undefined ||
          typeof value['requestId'] === 'string') &&
        typeof value['tool'] === 'string' &&
        typeof value['description'] === 'string'
      )
    case 'decision':
      return (
        typeof value['seq'] === 'number' &&
        typeof value['title'] === 'string' &&
        typeof value['why'] === 'string' &&
        Array.isArray(value['considered']) &&
        value['considered'].every((item) => typeof item === 'string') &&
        typeof value['easyToChange'] === 'string' &&
        (value['review'] === null ||
          value['review'] === 'ok' ||
          value['review'] === 'change') &&
        typeof value['createdAt'] === 'string'
      )
    default:
      return false
  }
}

function isBackgroundTask(value: unknown): value is BackgroundTask {
  return (
    isRecord(value) &&
    typeof value['id'] === 'string' &&
    typeof value['description'] === 'string' &&
    typeof value['elapsedSeconds'] === 'number' &&
    (value['kind'] === 'subagent' || value['kind'] === 'background_task') &&
    (value['agentType'] === undefined || typeof value['agentType'] === 'string')
  )
}

function isSkillSuggestion(value: unknown): value is SkillSuggestion {
  return (
    isRecord(value) &&
    typeof value['name'] === 'string' &&
    typeof value['description'] === 'string'
  )
}

export function isConversationResponse(
  value: unknown,
): value is ConversationResponse {
  return (
    isRecord(value) &&
    Array.isArray(value['messages']) &&
    value['messages'].every(isChatItem) &&
    Array.isArray(value['backgroundTasks']) &&
    value['backgroundTasks'].every(isBackgroundTask) &&
    (value['skills'] === undefined ||
      (Array.isArray(value['skills']) &&
        value['skills'].every(isSkillSuggestion))) &&
    typeof value['start'] === 'number' &&
    Number.isInteger(value['start']) &&
    value['start'] >= 0 &&
    typeof value['total'] === 'number' &&
    Number.isInteger(value['total']) &&
    value['total'] >= value['start'] + value['messages'].length &&
    Array.isArray(value['decisions']) &&
    value['decisions'].every(
      (decision) => isChatItem(decision) && decision.type === 'decision',
    ) &&
    typeof value['hasUnansweredProposal'] === 'boolean' &&
    (value['activeQuestionIndex'] === null ||
      (typeof value['activeQuestionIndex'] === 'number' &&
        Number.isInteger(value['activeQuestionIndex']) &&
        value['activeQuestionIndex'] >= 0 &&
        value['activeQuestionIndex'] < value['total']))
  )
}

export function isSessionActions(value: unknown): value is SessionActions {
  return (
    isRecord(value) &&
    typeof value['message'] === 'boolean' &&
    typeof value['answerQuestion'] === 'boolean' &&
    typeof value['interrupt'] === 'boolean' &&
    typeof value['openInTerminal'] === 'boolean' &&
    (value['permission'] === undefined ||
      (isRecord(value['permission']) &&
        typeof value['permission']['approve'] === 'boolean' &&
        typeof value['permission']['reject'] === 'boolean'))
  )
}

export function isSessionTreeResponse(
  value: unknown,
): value is { revision: number; sessions: SessionTreeNode[] } {
  return (
    isRecord(value) &&
    typeof value['revision'] === 'number' &&
    Number.isInteger(value['revision']) &&
    Array.isArray(value['sessions']) &&
    value['sessions'].every(
      (session) =>
        isRecord(session) &&
        typeof session['id'] === 'string' &&
        (session['parentId'] === null ||
          typeof session['parentId'] === 'string') &&
        typeof session['category'] === 'string' &&
        typeof session['isActive'] === 'boolean' &&
        (session['kind'] === null || typeof session['kind'] === 'string') &&
        typeof session['title'] === 'string' &&
        (session['preview'] === null ||
          typeof session['preview'] === 'string') &&
        typeof session['elapsedSeconds'] === 'number' &&
        Number.isInteger(session['elapsedSeconds']) &&
        (session['task'] === null || isSessionTask(session['task'])) &&
        typeof session['repo'] === 'string' &&
        typeof session['decisionCount'] === 'number' &&
        Number.isInteger(session['decisionCount']) &&
        (session['pullRequest'] === null ||
          isPullRequest(session['pullRequest'])),
    )
  )
}

export function isSessionPullRequestListResponse(
  value: unknown,
): value is SessionPullRequestListResponse {
  return (
    isRecord(value) &&
    (value['error'] === null || typeof value['error'] === 'string') &&
    Array.isArray(value['pullRequests']) &&
    value['pullRequests'].every(
      (pullRequest) =>
        isRecord(pullRequest) &&
        typeof pullRequest['number'] === 'number' &&
        Number.isInteger(pullRequest['number']) &&
        pullRequest['number'] > 0 &&
        typeof pullRequest['title'] === 'string' &&
        typeof pullRequest['url'] === 'string' &&
        (pullRequest['state'] === null ||
          pullRequest['state'] === 'Open' ||
          pullRequest['state'] === 'Closed' ||
          pullRequest['state'] === 'Merged') &&
        (pullRequest['ciState'] === null ||
          pullRequest['ciState'] === 'Waiting' ||
          pullRequest['ciState'] === 'InProgress' ||
          pullRequest['ciState'] === 'Successful' ||
          pullRequest['ciState'] === 'Unknown') &&
        (pullRequest['reviewStatus'] === 'done' ||
          pullRequest['reviewStatus'] === 'user' ||
          pullRequest['reviewStatus'] === 'working' ||
          pullRequest['reviewStatus'] === 'waiting' ||
          pullRequest['reviewStatus'] === 'not_started'),
    )
  )
}
