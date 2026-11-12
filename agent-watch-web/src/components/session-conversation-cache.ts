import {
  createContext,
  createElement,
  type PropsWithChildren,
  useCallback,
  useContext,
  useMemo,
  useState,
} from 'react'

import type { ConversationResponse } from '#types'

const MAX_CACHED_CONVERSATIONS = 8

export type CachedConversation = {
  conversation: ConversationResponse
  messagesLoadedAt: number
}

type SessionConversationCacheValue = {
  conversations: ReadonlyMap<string, CachedConversation>
  setCachedConversation: (sessionId: string, cached: CachedConversation) => void
  touchCachedConversation: (sessionId: string) => void
  clearCachedConversation: (sessionId: string) => void
}

const SessionConversationCacheContext =
  createContext<SessionConversationCacheValue | null>(null)

export function SessionConversationCacheProvider({
  children,
}: PropsWithChildren) {
  const [conversations, setConversations] = useState(
    () => new Map<string, CachedConversation>(),
  )
  const setCachedConversation = useCallback(
    (sessionId: string, cached: CachedConversation) => {
      setConversations((current) => {
        const next = new Map(current)
        next.delete(sessionId)
        next.set(sessionId, cached)
        if (next.size > MAX_CACHED_CONVERSATIONS) {
          const oldestSessionId = next.keys().next().value
          if (oldestSessionId !== undefined) next.delete(oldestSessionId)
        }
        return next
      })
    },
    [],
  )
  const touchCachedConversation = useCallback((sessionId: string) => {
    setConversations((current) => {
      const cached = current.get(sessionId)
      const mostRecentlyUsedSessionId = [...current.keys()].at(-1)
      if (cached === undefined || mostRecentlyUsedSessionId === sessionId) {
        return current
      }
      const next = new Map(current)
      next.delete(sessionId)
      next.set(sessionId, cached)
      return next
    })
  }, [])
  const clearCachedConversation = useCallback((sessionId: string) => {
    setConversations((current) => {
      if (!current.has(sessionId)) return current
      const next = new Map(current)
      next.delete(sessionId)
      return next
    })
  }, [])
  const value = useMemo(
    () => ({
      conversations,
      setCachedConversation,
      touchCachedConversation,
      clearCachedConversation,
    }),
    [
      clearCachedConversation,
      conversations,
      setCachedConversation,
      touchCachedConversation,
    ],
  )

  return createElement(
    SessionConversationCacheContext.Provider,
    { value },
    children,
  )
}

export function useSessionConversationCache() {
  return useContext(SessionConversationCacheContext)
}
