import type { ConversationResponse } from '#types'

const MAX_CACHED_CONVERSATIONS = 8
const conversations = new Map<string, ConversationResponse>()

export function getCachedConversation(
  sessionId: string,
): ConversationResponse | undefined {
  const cached = conversations.get(sessionId)
  if (cached === undefined) return undefined
  conversations.delete(sessionId)
  conversations.set(sessionId, cached)
  return cached
}

export function setCachedConversation(
  sessionId: string,
  conversation: ConversationResponse,
): void {
  conversations.delete(sessionId)
  conversations.set(sessionId, conversation)
  if (conversations.size > MAX_CACHED_CONVERSATIONS) {
    const oldestSessionId = conversations.keys().next().value
    if (oldestSessionId !== undefined) conversations.delete(oldestSessionId)
  }
}

export function clearCachedConversation(sessionId: string): void {
  conversations.delete(sessionId)
}
