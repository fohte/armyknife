import {
  createContext,
  createElement,
  type PropsWithChildren,
  useContext,
  useState,
} from 'react'

import type { ConversationResponse } from '#types'

const MAX_CACHED_CONVERSATIONS = 8

type CachedConversation = {
  conversation: ConversationResponse
  messagesLoadedAt: number
}

type SessionConversationCacheValue = {
  getCachedConversation: (sessionId: string) => CachedConversation | undefined
  setCachedConversation: (sessionId: string, cached: CachedConversation) => void
  clearCachedConversation: (sessionId: string) => void
}

const SessionConversationCacheContext =
  createContext<SessionConversationCacheValue | null>(null)

function createSessionConversationCache(): SessionConversationCacheValue {
  const conversations = new Map<string, CachedConversation>()

  return {
    getCachedConversation(sessionId) {
      const cached = conversations.get(sessionId)
      if (cached === undefined) return undefined
      conversations.delete(sessionId)
      conversations.set(sessionId, cached)
      return cached
    },
    setCachedConversation(sessionId, cached) {
      conversations.delete(sessionId)
      conversations.set(sessionId, cached)
      if (conversations.size > MAX_CACHED_CONVERSATIONS) {
        const oldestSessionId = conversations.keys().next().value
        if (oldestSessionId !== undefined) conversations.delete(oldestSessionId)
      }
    },
    clearCachedConversation(sessionId) {
      conversations.delete(sessionId)
    },
  }
}

export function SessionConversationCacheProvider({
  children,
}: PropsWithChildren) {
  const [cache] = useState(() => createSessionConversationCache())

  return createElement(
    SessionConversationCacheContext.Provider,
    { value: cache },
    children,
  )
}

export function useSessionConversationCache() {
  return useContext(SessionConversationCacheContext)
}
