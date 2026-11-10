import { ResultAsync } from 'neverthrow'
import { useEffect, useRef, useState } from 'react'

import type { ComposerAttachment } from '#components/message-composer'

const MAX_IMAGE_BYTES = 10 * 1024 * 1024
const PREVIEW_MIME_TYPES = new Set([
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
])

type SelectedAttachment = ComposerAttachment & { file: File }

type ComposerMessageOptions = {
  draft: string
  messageEnabled: boolean
  externalSending: boolean
  clearDraftIfUnchanged: (sentDraft: string) => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
}

function encodeAttachment(file: File) {
  if (file.size > MAX_IMAGE_BYTES) {
    return Promise.resolve({ mimeType: file.type, size: file.size, data: '' })
  }
  return file.arrayBuffer().then((buffer) => {
    const bytes = new Uint8Array(buffer)
    let binary = ''
    for (let offset = 0; offset < bytes.length; offset += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000))
    }
    return {
      mimeType: file.type,
      size: file.size,
      data: window.btoa(binary),
    }
  })
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}

export function useComposerMessage({
  draft,
  messageEnabled,
  externalSending,
  clearDraftIfUnchanged,
  onAction,
}: ComposerMessageOptions) {
  const [attachments, setAttachments] = useState<SelectedAttachment[]>([])
  const [sendPending, setSendPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const sendingRef = useRef(false)
  const attachmentsRef = useRef(attachments)
  attachmentsRef.current = attachments
  const isSending = externalSending || sendPending

  useEffect(
    () => () => {
      attachmentsRef.current.forEach((attachment) => {
        if (attachment.previewUrl !== null) {
          URL.revokeObjectURL(attachment.previewUrl)
        }
      })
    },
    [],
  )

  const onPasteFiles = (files: File[]) => {
    const added = files.map((file) => ({
      id: crypto.randomUUID(),
      name: file.name || 'Pasted file',
      file,
      previewUrl: PREVIEW_MIME_TYPES.has(file.type)
        ? URL.createObjectURL(file)
        : null,
    }))
    setAttachments((current) => [...current, ...added])
    setError(null)
  }

  const removeAttachment = (id: string) => {
    const removed = attachments.find((attachment) => attachment.id === id)
    if (removed?.previewUrl !== null && removed?.previewUrl !== undefined) {
      URL.revokeObjectURL(removed.previewUrl)
    }
    setAttachments(attachments.filter((attachment) => attachment.id !== id))
  }

  const removeAttachments = (ids: readonly string[]) => {
    const removedIds = new Set(ids)
    attachmentsRef.current
      .filter((attachment) => removedIds.has(attachment.id))
      .forEach((attachment) => {
        if (attachment.previewUrl !== null) {
          URL.revokeObjectURL(attachment.previewUrl)
        }
      })
    setAttachments((current) =>
      current.filter((attachment) => !removedIds.has(attachment.id)),
    )
  }

  const sendMessage = () => {
    if (
      (draft.trim() === '' && attachments.length === 0) ||
      !messageEnabled ||
      isSending ||
      sendingRef.current
    )
      return
    const sentDraft = draft
    const sentAttachments = attachments
    sendingRef.current = true
    setSendPending(true)
    const finishSending = () => {
      sendingRef.current = false
      setSendPending(false)
    }
    if (sentAttachments.length === 0) {
      void onAction('messages', { content: sentDraft }).then((succeeded) => {
        if (succeeded) clearDraftIfUnchanged(sentDraft)
        finishSending()
      })
      return
    }
    void ResultAsync.fromPromise(
      Promise.all(sentAttachments.map(({ file }) => encodeAttachment(file))),
      toError,
    ).match(
      async (images) => {
        const succeeded = await onAction('messages', {
          content: sentDraft,
          images,
        })
        if (succeeded) {
          clearDraftIfUnchanged(sentDraft)
          removeAttachments(sentAttachments.map(({ id }) => id))
          setError(null)
        }
        finishSending()
      },
      (requestError) => {
        setError(requestError.message)
        finishSending()
      },
    )
  }

  return {
    attachments,
    error,
    isSending,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  }
}
