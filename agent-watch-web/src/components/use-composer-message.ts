import { ResultAsync } from 'neverthrow'
import { useEffect, useRef, useState } from 'react'

import type { ComposerAttachment } from '#components/message-composer'

const MAX_ATTACHMENT_BYTES = 50 * 1024 * 1024
const MAX_ATTACHMENTS_PER_MESSAGE = 16
const BASE64_CHUNK_BYTES = 0x6000
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

async function encodeAttachment(file: File) {
  let data = ''
  for (let offset = 0; offset < file.size; offset += BASE64_CHUNK_BYTES) {
    const bytes = new Uint8Array(
      await file.slice(offset, offset + BASE64_CHUNK_BYTES).arrayBuffer(),
    )
    let binary = ''
    for (
      let chunkOffset = 0;
      chunkOffset < bytes.length;
      chunkOffset += 0x8000
    ) {
      binary += String.fromCharCode(
        ...bytes.subarray(chunkOffset, chunkOffset + 0x8000),
      )
    }
    data += window.btoa(binary)
  }
  return {
    name: file.name || 'Pasted file',
    mimeType: file.type,
    size: file.size,
    data,
  }
}

async function encodeAttachments(attachments: readonly SelectedAttachment[]) {
  const encoded = []
  for (const { file } of attachments) {
    encoded.push(await encodeAttachment(file))
  }
  return encoded
}

function attachmentLimitError(attachments: readonly SelectedAttachment[]) {
  let totalBytes = 0
  for (const [index, attachment] of attachments.entries()) {
    if (index >= MAX_ATTACHMENTS_PER_MESSAGE) {
      return `${attachment.name} exceeds the limit of ${String(MAX_ATTACHMENTS_PER_MESSAGE)} attachments per message`
    }
    if (totalBytes + attachment.file.size > MAX_ATTACHMENT_BYTES) {
      return `${attachment.name} would exceed the 50 MiB attachment limit`
    }
    totalBytes += attachment.file.size
  }
  return null
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
  const limitError = attachmentLimitError(attachments)

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
      size: file.size,
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
    if (attachmentLimitError(attachments) !== null) return
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
      encodeAttachments(sentAttachments),
      toError,
    ).match(
      async (encodedAttachments) => {
        const succeeded = await onAction('messages', {
          content: sentDraft,
          attachments: encodedAttachments,
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
    error: limitError ?? error,
    isSending,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  }
}
