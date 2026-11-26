import type { ChatItem, Session } from '#types'

function isVisibleConversationMessage(
  message: ChatItem,
  sessionCategory: Session['category'],
  isCompacting: boolean,
): boolean {
  return (
    message.type !== 'tool_in_progress' ||
    (sessionCategory !== 'in_progress' && !isCompacting)
  )
}

export function getVisibleConversationMessageEntries(
  messages: readonly ChatItem[],
  sessionCategory: Session['category'],
  isCompacting: boolean,
): { message: ChatItem; index: number }[] {
  return messages.flatMap((message, index) =>
    isVisibleConversationMessage(message, sessionCategory, isCompacting)
      ? [{ message, index }]
      : [],
  )
}

export function getConversationTimelineEndIndex(
  messages: readonly ChatItem[],
  start: number,
  sessionCategory: Session['category'],
  isCompacting: boolean,
): number {
  const lastVisibleIndex = messages.findLastIndex((message) =>
    isVisibleConversationMessage(message, sessionCategory, isCompacting),
  )
  return start + lastVisibleIndex + 1
}
