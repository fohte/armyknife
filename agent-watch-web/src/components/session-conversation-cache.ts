import type { ConversationResponse } from '#types'

const MAX_CACHED_CONVERSATIONS = 8
export type CachedConversation = {
  conversation: ConversationResponse
  messagesLoadedAt: number
}

const conversations = new Map<string, CachedConversation>()

export function getCachedConversation(
  sessionId: string,
): CachedConversation | undefined {
  const cached = conversations.get(sessionId)
  if (cached === undefined) return undefined
  conversations.delete(sessionId)
  conversations.set(sessionId, cached)
  return cached
}

export function setCachedConversation(
  sessionId: string,
  cached: CachedConversation,
): void {
  conversations.delete(sessionId)
  conversations.set(sessionId, cached)
  if (conversations.size > MAX_CACHED_CONVERSATIONS) {
    const oldestSessionId = conversations.keys().next().value
    if (oldestSessionId !== undefined) conversations.delete(oldestSessionId)
  }
}

export function clearCachedConversation(sessionId: string): void {
  conversations.delete(sessionId)
}

export function clearCachedConversations(): void {
  conversations.clear()
}
