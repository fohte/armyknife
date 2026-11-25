import { useRef, useState } from 'react'

import { useAttachments } from '#components/use-attachments'

export { clearSessionAttachments } from '#components/use-attachments'

type ComposerMessageOptions = {
  sessionId: string
  draft: string
  messageEnabled: boolean
  externalSending: boolean
  clearDraft: () => void
  restoreDraftIfEmpty: (sentDraft: string) => void
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
  clearDraft,
  restoreDraftIfEmpty,
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
    clearDraft()
    sendingRef.current = true
    setSendPending(true)
    const finishSending = () => {
      sendingRef.current = false
      setSendPending(false)
    }
    try {
      const succeeded = await attachmentState.sendWithAttachments(
        async (encodedAttachments) => {
          const succeeded = await onAction('messages', {
            content: sentDraft,
            ...(encodedAttachments.length === 0
              ? {}
              : { attachments: encodedAttachments }),
          })
          if (succeeded) {
            if (encodedAttachments.length > 0) {
              attachmentState.removeAttachments(attachments.map(({ id }) => id))
            }
          }
          return succeeded
        },
      )
      if (!succeeded) restoreDraftIfEmpty(sentDraft)
    } catch (error) {
      restoreDraftIfEmpty(sentDraft)
      throw error
    } finally {
      finishSending()
    }
  }

  return {
    ...attachmentState,
    isSending,
    sendMessage,
  }
}
