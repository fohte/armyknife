import { ResultAsync } from 'neverthrow'
import { useRef, useState } from 'react'

import { useAttachments } from '#components/use-attachments'

export { clearSessionAttachments } from '#components/use-attachments'

type ComposerMessageOptions = {
  sessionId: string
  draft: string
  messageEnabled: boolean
  externalSending: boolean
  clearDraftForSend: () => number
  settleDraftSend: (
    sentDraft: string,
    revision: number,
    succeeded: boolean,
  ) => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
}

export function useComposerMessage({
  sessionId,
  draft,
  messageEnabled,
  externalSending,
  clearDraftForSend,
  settleDraftSend,
  onAction,
}: ComposerMessageOptions) {
  const attachmentState = useAttachments(sessionId)
  const [sendPending, setSendPending] = useState(false)
  const sendingRef = useRef(false)
  const isSending = externalSending || sendPending

  const sendMessage = async () => {
    const { attachments, isProcessingAttachments, hasLimitError } =
      attachmentState
    if (hasLimitError) return
    if (
      (draft.trim() === '' && attachments.length === 0) ||
      !messageEnabled ||
      isSending ||
      sendingRef.current ||
      isProcessingAttachments
    ) {
      return
    }
    const sentDraft = draft
    const clearedDraftRevision = clearDraftForSend()
    sendingRef.current = true
    setSendPending(true)
    const finishSending = () => {
      sendingRef.current = false
      setSendPending(false)
    }
    let succeeded = false
    try {
      succeeded = await ResultAsync.fromPromise(
        attachmentState.sendWithAttachments(async (encodedAttachments) => {
          const succeeded = await onAction('messages', {
            content: sentDraft,
            ...(encodedAttachments.length === 0
              ? {}
              : { attachments: encodedAttachments }),
          })
          if (succeeded && encodedAttachments.length > 0) {
            attachmentState.removeAttachments(attachments.map(({ id }) => id))
          }
          return succeeded
        }),
        (error: unknown) => error,
      ).match(
        (result) => result,
        () => false,
      )
    } finally {
      settleDraftSend(sentDraft, clearedDraftRevision, succeeded)
      finishSending()
    }
  }

  return {
    ...attachmentState,
    isSending,
    sendMessage,
  }
}
