import type { SessionOperation } from '#session-operation-types'
import type {
  ChatItem,
  ConversationResponse,
  Session,
  SessionActions,
} from '#types'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function applySessionOperation(
  sessions: readonly Session[],
  operation: SessionOperation,
): Session[] {
  if (operation.type === 'close') {
    return sessions.filter((session) => session.id !== operation.sessionId)
  }
  return sessions.map((session) => {
    if (session.id !== operation.sessionId) return session
    switch (operation.type) {
      case 'today':
        return { ...session, isToday: operation.isToday }
      case 'later':
        return {
          ...session,
          categoryWhenNotLater:
            session.category === 'later'
              ? (session.categoryWhenNotLater ?? 'needs_action')
              : session.category,
          category: 'later',
          laterUntil: operation.expectedDeadline,
        }
      case 'back_now': {
        if (
          session.category !== 'later' &&
          (session.laterUntil === null || session.laterUntil === undefined) &&
          session.categoryWhenNotLater === undefined
        ) {
          return session
        }
        const next = { ...session }
        delete next.categoryWhenNotLater
        return {
          ...next,
          category: session.categoryWhenNotLater ?? 'needs_action',
          laterUntil: null,
        }
      }
      case 'primary_task': {
        const task = session.tasks?.find(({ id }) => id === operation.taskId)
        return task === undefined ? session : { ...session, task }
      }
      case 'title':
        return operation.title.trim() === ''
          ? session
          : { ...session, title: operation.title.trim() }
      default:
        return session
    }
  })
}

function updateChatItems(
  messages: ChatItem[],
  update: (message: ChatItem) => ChatItem | null,
) {
  return messages.flatMap((message) => {
    const next = update(message)
    return next === null ? [] : [next]
  })
}

export function applyConversationOperation(
  conversation: ConversationResponse,
  operation: SessionOperation,
): ConversationResponse {
  let messages = conversation.messages
  let total = conversation.total
  switch (operation.type) {
    case 'message': {
      const messageCountBefore = operation.messageCountBefore
      const hasServerMessage =
        messageCountBefore !== undefined &&
        conversation.messages.some(
          (message, index) =>
            matchesUserMessage(message, operation) &&
            conversation.start + index >= messageCountBefore,
        )
      if (!hasServerMessage) {
        const content = []
        if (operation.content !== '') {
          content.push({ type: 'text' as const, text: operation.content })
        }
        for (const attachment of operation.attachments ?? []) {
          if (!isRecord(attachment)) continue
          const name = attachment['name']
          const mimeType = attachment['mimeType']
          const data = attachment['data']
          if (typeof name !== 'string') continue
          if (
            typeof mimeType === 'string' &&
            ['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(
              mimeType,
            ) &&
            typeof data === 'string'
          ) {
            content.push({
              type: 'image' as const,
              source: `data:${mimeType};base64,${data}`,
              available: true,
            })
          } else {
            content.push({ type: 'file' as const, name })
          }
        }
        messages = [...messages, { type: 'queued_user', content }]
        total += 1
      }
      break
    }
    case 'answer':
      messages = updateChatItems(messages, (message) => {
        if (
          message.type !== 'question' ||
          message.requestId !== operation.requestId
        ) {
          return message
        }
        if (operation.skip === true) return null
        return {
          ...message,
          questions: message.questions.map((question) => ({
            ...question,
            answer: operation.answers?.[question.question] ?? question.answer,
          })),
        }
      })
      break
    case 'decision_ok':
    case 'decision_change':
      messages = updateChatItems(messages, (message) =>
        message.type === 'decision' && message.seq === operation.seq
          ? {
              ...message,
              review: operation.type === 'decision_ok' ? 'ok' : 'change',
            }
          : message,
      )
      break
    case 'proposal_answer':
      messages = updateChatItems(messages, (message) =>
        message.type === 'proposal' && message.seq === operation.seq
          ? {
              ...message,
              answer: { choice: operation.choice, comment: operation.comment },
              resolved: true,
            }
          : message,
      )
      break
    case 'todo_complete':
      messages = updateChatItems(messages, (message) =>
        message.type === 'todo' && message.seq === operation.seq
          ? { ...message, completed: true, comment: operation.comment }
          : message,
      )
      break
    case 'permission_retry':
      messages = updateChatItems(messages, (message) =>
        message.type === 'permission_denied' && message.seq === operation.seq
          ? { ...message, resolved: true }
          : message,
      )
      break
    default:
      break
  }
  const decisions = messages.filter(
    (message): message is Extract<ChatItem, { type: 'decision' }> =>
      message.type === 'decision',
  )
  return {
    ...conversation,
    messages,
    total,
    decisions,
    hasUnansweredProposal: messages.some(
      (message) =>
        message.type === 'proposal' &&
        message.answer === null &&
        !message.resolved,
    ),
    activeQuestionIndex: (() => {
      const index = messages.findLastIndex(
        (message) => message.type === 'question',
      )
      return index < 0 ? null : conversation.start + index
    })(),
  }
}

export function isConversationOperationConfirmed(
  conversation: ConversationResponse,
  operation: SessionOperation,
  actions: SessionActions | null,
) {
  switch (operation.type) {
    case 'message':
      return conversation.messages.some(
        (message, index) =>
          matchesUserMessage(message, operation) &&
          (operation.messageCountBefore === undefined ||
            conversation.start + index >= operation.messageCountBefore),
      )
    case 'answer': {
      const question = conversation.messages.find(
        (message) =>
          message.type === 'question' &&
          message.requestId === operation.requestId,
      )
      if (operation.skip === true) return question === undefined
      return (
        question === undefined ||
        (question.type === 'question' &&
          question.questions.every(
            (item) => item.answer === operation.answers?.[item.question],
          ))
      )
    }
    case 'decision_ok':
      return conversation.messages.some(
        (message) =>
          message.type === 'decision' &&
          message.seq === operation.seq &&
          message.review === 'ok',
      )
    case 'decision_change':
      return conversation.messages.some(
        (message) =>
          message.type === 'decision' &&
          message.seq === operation.seq &&
          message.review === 'change',
      )
    case 'proposal_answer':
      return conversation.messages.some(
        (message) =>
          message.type === 'proposal' &&
          message.seq === operation.seq &&
          message.answer?.choice === operation.choice &&
          message.answer.comment === operation.comment &&
          message.resolved,
      )
    case 'todo_complete':
      return conversation.messages.some(
        (message) =>
          message.type === 'todo' &&
          message.seq === operation.seq &&
          message.completed &&
          message.comment === operation.comment,
      )
    case 'permission':
      return (
        (actions !== null &&
          !conversation.messages.some(
            (message) =>
              message.type === 'permission' &&
              message.requestId === operation.requestId,
          )) ||
        (operation.action === 'approve'
          ? actions?.permission?.approve !== true
          : actions?.permission?.reject !== true)
      )
    case 'permission_retry':
      return conversation.messages.some(
        (message) =>
          message.type === 'permission_denied' &&
          message.seq === operation.seq &&
          message.resolved,
      )
    default:
      return false
  }
}

function matchesUserMessage(
  message: ChatItem,
  operation: Extract<SessionOperation, { type: 'message' }>,
) {
  const expectedAttachmentCount = operation.attachments?.length ?? 0
  if (message.type === 'user') {
    return expectedAttachmentCount === 0 && message.text === operation.content
  }
  if (message.type !== 'queued_user' && message.type !== 'user_with_content') {
    return false
  }
  const text = message.content
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
  const actualAttachmentCount = message.content.filter(
    (part) => part.type === 'image' || part.type === 'file',
  ).length
  return (
    text === operation.content &&
    actualAttachmentCount === expectedAttachmentCount
  )
}

export function isSessionOperationConfirmed(
  operation: SessionOperation,
  sessions: readonly Session[],
) {
  const session = sessions.find(({ id }) => id === operation.sessionId)
  switch (operation.type) {
    case 'today':
      return (session?.isToday ?? false) === operation.isToday
    case 'later':
      return (
        session?.category === 'later' &&
        (session.laterUntil ?? null) === operation.expectedDeadline
      )
    case 'back_now':
      return session !== undefined && session.category !== 'later'
    case 'close':
      return session === undefined
    case 'primary_task':
      return session?.task?.id === operation.taskId
    case 'title':
      return operation.title.trim() === ''
        ? false
        : session?.title === operation.title.trim()
    default:
      return true
  }
}

export function isSessionListOperation(operation: SessionOperation) {
  return (
    operation.type === 'today' ||
    operation.type === 'later' ||
    operation.type === 'back_now' ||
    operation.type === 'close' ||
    operation.type === 'primary_task' ||
    operation.type === 'title'
  )
}

export function requiresStateConfirmation(operation: SessionOperation) {
  if (operation.type === 'title' && operation.title.trim() === '') return false
  return (
    isSessionListOperation(operation) ||
    operation.type === 'message' ||
    operation.type === 'answer' ||
    operation.type === 'decision_ok' ||
    operation.type === 'decision_change' ||
    operation.type === 'proposal_answer' ||
    operation.type === 'todo_complete' ||
    operation.type === 'permission' ||
    operation.type === 'permission_retry'
  )
}
