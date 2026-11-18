import { useCallback, useEffect, useRef, useState } from 'react'

type ComposerFocusRequest = {
  sessionId: string
  requestId: number
}

export function useComposerFocus({
  isMobile,
  selectedId,
}: {
  isMobile: boolean
  selectedId: string | null
}) {
  const [composerFocusRequest, setComposerFocusRequest] =
    useState<ComposerFocusRequest | null>(null)
  const requestId = useRef(0)
  const requestComposerFocus = useCallback(
    (sessionId: string) => {
      if (isMobile) return
      requestId.current += 1
      setComposerFocusRequest({ sessionId, requestId: requestId.current })
    },
    [isMobile],
  )
  const handleComposerFocusRequest = useCallback((handledRequestId: number) => {
    setComposerFocusRequest((current) =>
      current?.requestId === handledRequestId ? null : current,
    )
  }, [])
  const focusComposer = useCallback(() => {
    if (isMobile) return false
    const textarea = document.querySelector<HTMLTextAreaElement>(
      '.conversation-composer textarea[aria-label="Message"]',
    )
    const composer = textarea?.closest<HTMLFormElement>(
      '.conversation-composer',
    )
    if (
      textarea === null ||
      textarea.disabled ||
      composer?.dataset['actionsSettled'] !== 'true'
    ) {
      return false
    }
    textarea.focus()
    return document.activeElement === textarea
  }, [isMobile])

  useEffect(() => {
    if (
      composerFocusRequest !== null &&
      composerFocusRequest.sessionId !== selectedId
    ) {
      setComposerFocusRequest(null)
    }
  }, [composerFocusRequest, selectedId])

  return {
    composerFocusRequest,
    requestComposerFocus,
    handleComposerFocusRequest,
    focusComposer,
  }
}
