import { Result } from 'neverthrow'
import { useRef, useState } from 'react'

function readComposerDraft(key: string) {
  return Result.fromThrowable(
    () => window.localStorage.getItem(key),
    (error: unknown) => error,
  )().match(
    (draft) => ({ draft: draft ?? '', storageAvailable: true }),
    () => ({ draft: '', storageAvailable: false }),
  )
}

function writeComposerDraft(key: string, draft: string) {
  return Result.fromThrowable(
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
