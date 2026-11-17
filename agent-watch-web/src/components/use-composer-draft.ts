import { Result } from 'neverthrow'
import { useCallback, useRef, useState } from 'react'

const fallbackDrafts = new Map<string, string>()

function readDraft(key: string, initialDraft: string) {
  const fallbackDraft = fallbackDrafts.get(key)
  if (fallbackDraft !== undefined) {
    return { draft: fallbackDraft, storageAvailable: false }
  }

  return Result.fromThrowable(
    () => window.localStorage.getItem(key),
    (error: unknown) => error,
  )().match(
    (draft) => ({ draft: draft ?? initialDraft, storageAvailable: true }),
    () => ({ draft: initialDraft, storageAvailable: false }),
  )
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
  const draftRef = useRef(draft)

  const updateDraft = useCallback(
    (value: string) => {
      draftRef.current = value
      setDraft(value)
      setStorageAvailable(writeDraft(storageKey, value))
    },
    [storageKey],
  )

  const clearDraftIfUnchanged = useCallback(
    (sentDraft: string) => {
      if (draftRef.current === sentDraft) updateDraft('')
    },
    [updateDraft],
  )

  return { draft, updateDraft, clearDraftIfUnchanged, storageAvailable }
}

export function useComposerDraft(sessionId: string) {
  return useDraft(`agent-watch-web:composer-draft:${sessionId}`)
}
