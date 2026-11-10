import { Result } from 'neverthrow'
import { useRef, useState } from 'react'

function readComposerDraft(key: string) {
  return Result.fromThrowable(
    () => window.localStorage.getItem(key),
    (error: unknown) => error,
  )().match(
    (draft) => draft ?? '',
    () => '',
  )
}

function writeComposerDraft(key: string, draft: string) {
  Result.fromThrowable(
    () => {
      if (draft === '') {
        window.localStorage.removeItem(key)
      } else {
        window.localStorage.setItem(key, draft)
      }
    },
    (error: unknown) => error,
  )().match(
    () => undefined,
    () => undefined,
  )
}

export function useComposerDraft(sessionId: string) {
  const storageKey = `agent-watch-web:composer-draft:${sessionId}`
  const [draft, setDraft] = useState(() => readComposerDraft(storageKey))
  const draftRef = useRef(draft)

  const updateDraft = (value: string) => {
    draftRef.current = value
    setDraft(value)
    writeComposerDraft(storageKey, value)
  }

  const clearDraftIfUnchanged = (sentDraft: string) => {
    if (draftRef.current === sentDraft) updateDraft('')
  }

  return { draft, updateDraft, clearDraftIfUnchanged }
}
