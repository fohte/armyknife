import { useCallback, useEffect, useRef, useState } from 'react'

import {
  reuseEqualValue,
  shareUnchangedMessages,
} from '#components/conversation-message-sharing'
import {
  isConversationResponse,
  isSessionActions,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import { useSessionConversationCache } from '#components/session-conversation-cache'
import type { useSessionOperations } from '#session-operation-context'
import {
  isConversationOperationConfirmed,
  type SessionOperation,
} from '#session-operations'
import type { ConversationResponse, SessionActions } from '#types'

const EMPTY_CONVERSATION: ConversationResponse = {
  messages: [],
  backgroundTasks: [],
  skills: [],
  start: 0,
  total: 0,
  decisions: [],
  hasUnansweredProposal: false,
  activeQuestionIndex: null,
}
const INITIAL_MESSAGE_LIMIT = 100
const MAX_LOADED_MESSAGES = 300
const RETAINED_MESSAGE_COUNT = 200

type UseConversationPaneDataOptions = {
  sessionId: string
  refreshKey: number
  pendingOperations: ReturnType<
    typeof useSessionOperations
  >['pendingOperations']
  confirm: ReturnType<typeof useSessionOperations>['confirm']
}

export function useConversationPaneData({
  sessionId,
  refreshKey,
  pendingOperations,
  confirm,
}: UseConversationPaneDataOptions) {
  const sessionConversationCache = useSessionConversationCache()
  const setCachedConversation = sessionConversationCache?.setCachedConversation
  const [initialCachedConversation] = useState(() =>
    sessionConversationCache?.getCachedConversation(sessionId),
  )
  const [conversation, setConversation] = useState<ConversationResponse>(
    initialCachedConversation?.conversation ?? EMPTY_CONVERSATION,
  )
  const conversationRef = useRef(conversation)
  const conversationSessionId = useRef<string | null>(
    initialCachedConversation === undefined ? null : sessionId,
  )
  const [messagesLoadedAt, setMessagesLoadedAt] = useState<number | null>(
    initialCachedConversation?.messagesLoadedAt ?? null,
  )
  const messagesLoadedAtRef = useRef(messagesLoadedAt)
  const [actions, setActions] = useState<SessionActions | null>(null)
  const actionsRef = useRef<SessionActions | null>(null)
  const [actionsReady, setActionsReady] = useState(false)
  const [loading, setLoading] = useState(
    initialCachedConversation === undefined,
  )
  const [loadingOlder, setLoadingOlder] = useState(false)
  const loadingOlderRef = useRef(false)
  const [error, setError] = useState<string | null>(null)
  const currentSessionId = useRef(sessionId)
  currentSessionId.current = sessionId
  const requestGeneration = useRef(0)
  const actionsRequestGeneration = useRef(0)
  const olderRequestGeneration = useRef(0)
  const messagesError = useRef<string | null>(null)
  const actionsError = useRef<string | null>(null)
  const pendingOperationsRef = useRef(pendingOperations)
  pendingOperationsRef.current = pendingOperations

  const updateConversation = useCallback(
    (next: ConversationResponse) => {
      conversationRef.current = next
      setConversation(next)
      if (
        setCachedConversation &&
        conversationSessionId.current === sessionId
      ) {
        const loadedAt = messagesLoadedAtRef.current
        if (loadedAt !== null) {
          setCachedConversation(sessionId, {
            conversation: next,
            messagesLoadedAt: loadedAt,
          })
        }
      }
    },
    [sessionId, setCachedConversation],
  )
  const markMessagesLoaded = useCallback(() => {
    const loadedAt = Date.now()
    messagesLoadedAtRef.current = loadedAt
    setMessagesLoadedAt(loadedAt)
  }, [])
  const clearErrors = useCallback(() => {
    messagesError.current = null
    actionsError.current = null
    setError(null)
  }, [])
  const pruneOldMessages = useCallback(() => {
    const current = conversationRef.current
    if (current.messages.length <= MAX_LOADED_MESSAGES) return
    const dropCount = current.messages.length - RETAINED_MESSAGE_COUNT
    updateConversation({
      ...current,
      start: current.start + dropCount,
      stableStart: undefined,
      stableEnd: undefined,
      stableDigest: undefined,
      messages: current.messages.slice(dropCount),
    })
  }, [updateConversation])

  const confirmPendingOperations = useCallback(
    (
      confirmedConversation: ConversationResponse,
      confirmedActions: SessionActions | null,
    ) => {
      for (const pending of pendingOperationsRef.current) {
        if (
          pending.status === 'sent' &&
          pending.operation.sessionId === sessionId &&
          isConversationOperationConfirmed(
            confirmedConversation,
            pending.operation,
            confirmedActions,
          )
        ) {
          confirm(pending.operation)
        }
      }
    },
    [confirm, sessionId],
  )

  const refreshActions = useCallback(async () => {
    const generation = actionsRequestGeneration.current + 1
    actionsRequestGeneration.current = generation
    const result = await requestJson(
      `/api/sessions/${encodeURIComponent(sessionId)}/actions`,
      isSessionActions,
    )
    if (
      actionsRequestGeneration.current !== generation ||
      currentSessionId.current !== sessionId
    ) {
      return
    }
    if (result.isOk()) {
      actionsRef.current = result.value
      setActions((previous) => reuseEqualValue(previous, result.value))
      setActionsReady(true)
      actionsError.current = null
      confirmPendingOperations(conversationRef.current, result.value)
    } else {
      actionsError.current = result.error.message
    }
    setError(messagesError.current ?? actionsError.current)
  }, [confirmPendingOperations, sessionId])

  const refresh = useCallback(async () => {
    const generation = requestGeneration.current + 1
    requestGeneration.current = generation
    const sessionPath = `/api/sessions/${encodeURIComponent(sessionId)}`
    const loadedConversation =
      conversationSessionId.current === sessionId
        ? conversationRef.current
        : null
    const stableDigestQuery =
      loadedConversation !== null &&
      loadedConversation.stableStart === loadedConversation.start &&
      loadedConversation.stableEnd !== undefined &&
      loadedConversation.stableDigest !== undefined
        ? `&stableStart=${String(loadedConversation.stableStart)}&stableEnd=${String(loadedConversation.stableEnd)}&stableDigest=${loadedConversation.stableDigest}`
        : ''
    const messagesRequest = requestJson(
      loadedConversation === null
        ? `${sessionPath}/messages?limit=${String(INITIAL_MESSAGE_LIMIT)}`
        : `${sessionPath}/messages?from=${String(loadedConversation.start)}${stableDigestQuery}`,
      isConversationResponse,
    )
    void refreshActions()
    const messagesResult = await messagesRequest
    if (requestGeneration.current !== generation) return
    let refreshedConversation: ConversationResponse | null = null
    if (messagesResult.isOk()) {
      const alreadyLoaded = conversationSessionId.current === sessionId
      const previous = alreadyLoaded
        ? conversationRef.current
        : EMPTY_CONVERSATION
      const preserveLoadedPrefix =
        alreadyLoaded && previous.start < messagesResult.value.start
      const start = preserveLoadedPrefix
        ? previous.start
        : messagesResult.value.start
      const messages = preserveLoadedPrefix
        ? [
            ...previous.messages.slice(
              0,
              messagesResult.value.start - previous.start,
            ),
            ...messagesResult.value.messages,
          ]
        : messagesResult.value.messages
      refreshedConversation = {
        ...messagesResult.value,
        start,
        messages: shareUnchangedMessages(
          previous.messages,
          messages,
          previous.start,
          start,
        ),
      }
      conversationSessionId.current = sessionId
      markMessagesLoaded()
      updateConversation(refreshedConversation)
    }
    messagesError.current = messagesResult.isErr()
      ? messagesResult.error.message
      : null
    setError(messagesError.current ?? actionsError.current)
    confirmPendingOperations(
      refreshedConversation ?? conversationRef.current,
      actionsRef.current,
    )
    setLoading(false)
  }, [
    confirmPendingOperations,
    markMessagesLoaded,
    refreshActions,
    sessionId,
    updateConversation,
  ])

  const refreshForOperation = useCallback(
    async (operation: SessionOperation) => {
      await refresh()
      if (
        isConversationOperationConfirmed(
          conversationRef.current,
          operation,
          actionsRef.current,
        )
      ) {
        confirm(operation)
      }
    },
    [confirm, refresh],
  )

  const loadOlder = useCallback(() => {
    const current = conversationRef.current
    if (
      conversationSessionId.current !== sessionId ||
      current.start <= 0 ||
      loadingOlderRef.current
    ) {
      return
    }
    loadingOlderRef.current = true
    const generation = olderRequestGeneration.current + 1
    olderRequestGeneration.current = generation
    setLoadingOlder(true)
    void requestJson(
      `/api/sessions/${encodeURIComponent(sessionId)}/messages?before=${String(current.start)}&limit=${String(INITIAL_MESSAGE_LIMIT)}`,
      isConversationResponse,
    ).match(
      (response) => {
        if (olderRequestGeneration.current !== generation) return
        if (conversationSessionId.current !== sessionId) return
        const latest = conversationRef.current
        const messages = [...response.messages, ...latest.messages]
        markMessagesLoaded()
        updateConversation({
          ...response,
          stableStart: undefined,
          stableEnd: undefined,
          stableDigest: undefined,
          messages: shareUnchangedMessages(
            latest.messages,
            messages,
            latest.start,
            response.start,
          ),
        })
        clearErrors()
        loadingOlderRef.current = false
        setLoadingOlder(false)
      },
      (requestError) => {
        if (olderRequestGeneration.current !== generation) return
        setError(requestError.message)
        loadingOlderRef.current = false
        setLoadingOlder(false)
      },
    )
  }, [clearErrors, markMessagesLoaded, sessionId, updateConversation])

  useEffect(() => {
    void refresh()
    return () => {
      requestGeneration.current += 1
      actionsRequestGeneration.current += 1
    }
  }, [refresh, refreshKey])

  useEffect(() => {
    if (conversationSessionId.current !== sessionId) {
      updateConversation(EMPTY_CONVERSATION)
      messagesLoadedAtRef.current = null
      setMessagesLoadedAt(null)
      setLoadingOlder(false)
      loadingOlderRef.current = false
    }
    return () => {
      olderRequestGeneration.current += 1
    }
  }, [sessionId, updateConversation])

  return {
    actions,
    actionsReady,
    actionsRef,
    clearErrors,
    conversation,
    conversationRef,
    currentSessionId,
    error,
    loadOlder,
    loading,
    loadingOlder,
    messagesLoadedAt,
    pruneOldMessages,
    refresh,
    refreshForOperation,
    setError,
  }
}
