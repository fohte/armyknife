import { Result } from 'neverthrow'
import { useRef, useState } from 'react'

const fallbackDrafts = new Map<string, string>()

function readComposerDraft(key: string) {
  const fallbackDraft = fallbackDrafts.get(key)
  if (fallbackDraft !== undefined) {
    return { draft: fallbackDraft, storageAvailable: false }
  }

  return Result.fromThrowable(
    () => window.localStorage.getItem(key),
    (error: unknown) => error,
  )().match(
    (draft) => ({ draft: draft ?? '', storageAvailable: true }),
    () => ({ draft: '', storageAvailable: false }),
  )
}

function writeComposerDraft(key: string, draft: string) {
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
  const storageAvailable = writeComposerDraft(key, draft)
  return storageAvailable || fallbackDrafts.get(key) === draft
}

export function useComposerDraft(sessionId: string) {
  const storageKey = `agent-watch-web:composer-draft:${sessionId}`
  const [initialDraft] = useState(() => readComposerDraft(storageKey))
  const [draft, setDraft] = useState(initialDraft.draft)
  const [storageAvailable, setStorageAvailable] = useState(
    initialDraft.storageAvailable,
  )
  const draftRef = useRef(draft)

  const updateDraft = (value: string) => {
    draftRef.current = value
    setDraft(value)
    setStorageAvailable(writeComposerDraft(storageKey, value))
  }

  const clearDraftIfUnchanged = (sentDraft: string) => {
    if (draftRef.current === sentDraft) updateDraft('')
  }

  return { draft, updateDraft, clearDraftIfUnchanged, storageAvailable }
}
