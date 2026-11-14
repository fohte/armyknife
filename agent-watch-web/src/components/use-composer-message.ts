import { useRef, useState } from 'react'

import { useImageAttachments } from '#components/use-image-attachments'

export { clearSessionAttachments } from '#components/use-image-attachments'

type ComposerMessageOptions = {
  sessionId: string
  draft: string
  messageEnabled: boolean
  externalSending: boolean
  clearDraftIfUnchanged: (sentDraft: string) => void
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
  clearDraftIfUnchanged,
  onAction,
}: ComposerMessageOptions) {
  const attachmentState = useImageAttachments(sessionId)
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
    sendingRef.current = true
    setSendPending(true)
    const finishSending = () => {
      sendingRef.current = false
      setSendPending(false)
    }
    if (attachments.length === 0) {
      const succeeded = await onAction('messages', { content: sentDraft })
      if (succeeded) clearDraftIfUnchanged(sentDraft)
      finishSending()
      return
    }
    await attachmentState.encodeAttachments().match(
      async (encodedAttachments) => {
        const succeeded = await onAction('messages', {
          content: sentDraft,
          attachments: encodedAttachments,
        })
        if (succeeded) {
          clearDraftIfUnchanged(sentDraft)
          attachmentState.removeAttachments(attachments.map(({ id }) => id))
          attachmentState.setError(null)
        }
        finishSending()
      },
      (requestError) => {
        attachmentState.setError(requestError.message)
        finishSending()
      },
    )
  }

  return {
    ...attachmentState,
    isSending,
    sendMessage,
  }
}
