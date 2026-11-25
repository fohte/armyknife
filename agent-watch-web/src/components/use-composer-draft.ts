import { Result } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

import { useBeforeUnloadWarning } from '#components/use-beforeunload-warning'

const fallbackDrafts = new Map<string, string>()
const inMemoryDrafts = new Map<string, DraftSnapshot>()
const draftListeners = new Map<string, Set<(snapshot: DraftSnapshot) => void>>()
const pendingDraftSends = new Map<string, number>()

type DraftSnapshot = {
  draft: string
  storageAvailable: boolean
  revision: number
}

function readStoredDraft(key: string) {
  const fallbackDraft = fallbackDrafts.get(key)
  if (fallbackDraft !== undefined) {
    return { draft: fallbackDraft, storageAvailable: false }
  }

  return Result.fromThrowable(
    () => window.localStorage.getItem(key),
    (error: unknown) => error,
  )().match(
    (draft) => ({ draft, storageAvailable: true }),
    () => ({ draft: null, storageAvailable: false }),
  )
}

function readDraft(key: string, initialDraft: string) {
  const inMemoryDraft = inMemoryDrafts.get(key)
  if (inMemoryDraft !== undefined) return inMemoryDraft

  const storedDraft = readStoredDraft(key)
  return {
    draft: storedDraft.draft ?? initialDraft,
    storageAvailable: storedDraft.storageAvailable,
    revision: 0,
  }
}

function writeDraft(key: string, draft: string) {
  const storageAvailable = Result.fromThrowable(
    () => {
      if (draft === '') {
        window.localStorage.removeItem(key)
      } else {
        window.localStorage.setItem(key, draft)
      }
    },
    (error: unknown) => error,
  )().match(
    () => true,
    () => false,
  )
  if (storageAvailable) {
    fallbackDrafts.delete(key)
  } else {
    fallbackDrafts.set(key, draft)
  }
  return storageAvailable
}

function publishDraft(key: string, draft: string, storageAvailable: boolean) {
  const snapshot = {
    draft,
    storageAvailable,
    revision: (inMemoryDrafts.get(key)?.revision ?? 0) + 1,
  }
  inMemoryDrafts.set(key, snapshot)
  draftListeners.get(key)?.forEach((listener) => {
    listener(snapshot)
  })
  return snapshot
}

function releasePendingDraftSend(key: string) {
  const pending = pendingDraftSends.get(key) ?? 0
  if (pending <= 1) pendingDraftSends.delete(key)
  else pendingDraftSends.set(key, pending - 1)

  if (!draftListeners.has(key) && !pendingDraftSends.has(key)) {
    inMemoryDrafts.delete(key)
  }
}

export function writeComposerDraftForSession(sessionId: string, draft: string) {
  const key = `agent-watch-web:composer-draft:${sessionId}`
  const storageAvailable = writeDraft(key, draft)
  return storageAvailable || fallbackDrafts.get(key) === draft
}

export function cardDraftStorageKey(
  cardType: string,
  sessionId: string,
  cardId: string | number,
  field: string,
) {
  return `agent-watch-web:card-draft:${JSON.stringify([
    cardType,
    sessionId,
    cardId,
    field,
  ])}`
}

export function useDraft(storageKey: string, initialValue = '') {
  const [initialDraft] = useState(() => readDraft(storageKey, initialValue))
  const [draft, setDraft] = useState(initialDraft.draft)
  const [storageAvailable, setStorageAvailable] = useState(
    initialDraft.storageAvailable,
  )
  useBeforeUnloadWarning(draft !== '' && !storageAvailable)
  const draftRef = useRef(draft)

  useEffect(() => {
    const listener = (snapshot: DraftSnapshot) => {
      draftRef.current = snapshot.draft
      setDraft(snapshot.draft)
      setStorageAvailable(snapshot.storageAvailable)
    }
    const listeners = draftListeners.get(storageKey) ?? new Set()
    listeners.add(listener)
    draftListeners.set(storageKey, listeners)
    const inMemoryDraft = inMemoryDrafts.get(storageKey)
    if (inMemoryDraft !== undefined) listener(inMemoryDraft)

    return () => {
      listeners.delete(listener)
      if (listeners.size === 0) draftListeners.delete(storageKey)
      if (
        !draftListeners.has(storageKey) &&
        !pendingDraftSends.has(storageKey)
      ) {
        inMemoryDrafts.delete(storageKey)
      }
    }
  }, [storageKey])

  const updateDraft = useCallback(
    (value: string) => {
      const nextStorageAvailable = writeDraft(storageKey, value)
      publishDraft(storageKey, value, nextStorageAvailable)
      draftRef.current = value
      setDraft(value)
      setStorageAvailable(nextStorageAvailable)
    },
    [storageKey],
  )

  const clearDraftIfUnchanged = useCallback(
    (sentDraft: string) => {
      if (draftRef.current === sentDraft) updateDraft('')
    },
    [updateDraft],
  )

  const clearDraftForSend = useCallback(() => {
    pendingDraftSends.set(
      storageKey,
      (pendingDraftSends.get(storageKey) ?? 0) + 1,
    )
    const snapshot = publishDraft(storageKey, '', storageAvailable)
    draftRef.current = ''
    setDraft('')
    return snapshot.revision
  }, [storageAvailable, storageKey])

  const settleDraftSend = useCallback(
    (sentDraft: string, revision: number, succeeded: boolean) => {
      const currentDraft = inMemoryDrafts.get(storageKey)
      if (
        currentDraft?.revision === revision &&
        currentDraft.draft === '' &&
        readStoredDraft(storageKey).draft === sentDraft
      ) {
        if (succeeded) updateDraft('')
        else updateDraft(sentDraft)
      }
      releasePendingDraftSend(storageKey)
    },
    [storageKey, updateDraft],
  )

  return {
    draft,
    updateDraft,
    clearDraftForSend,
    settleDraftSend,
    clearDraftIfUnchanged,
    storageAvailable,
  }
}

export function useComposerDraft(sessionId: string) {
  return useDraft(`agent-watch-web:composer-draft:${sessionId}`)
}
