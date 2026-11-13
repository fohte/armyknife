import { ResultAsync } from 'neverthrow'
import { useRef, useState } from 'react'

import type { ComposerAttachment } from '#components/message-composer'
import { optimizeImageAttachment } from '#lib/image-attachment'

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

const attachmentsBySession = new Map<string, SelectedAttachment[]>()

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
  sessionId,
  draft,
  messageEnabled,
  externalSending,
  clearDraftIfUnchanged,
  onAction,
}: ComposerMessageOptions) {
  const [attachments, setAttachments] = useState<SelectedAttachment[]>(
    () => attachmentsBySession.get(sessionId) ?? [],
  )
  const [sendPending, setSendPending] = useState(false)
  const [isProcessingAttachments, setIsProcessingAttachments] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const sendingRef = useRef(false)
  const attachmentsRef = useRef(attachments)
  const pendingFileCountRef = useRef(0)
  const pasteQueueRef = useRef(Promise.resolve())
  attachmentsRef.current = attachments
  const isSending = externalSending || sendPending
  const limitError = attachmentLimitError(attachments)

  const updateAttachments = (nextAttachments: SelectedAttachment[]) => {
    attachmentsRef.current = nextAttachments
    attachmentsBySession.set(sessionId, nextAttachments)
    setAttachments(nextAttachments)
  }

  const onPasteFiles = (files: File[]) => {
    if (files.length === 0) return
    const availableOptimizationSlots = Math.max(
      0,
      MAX_ATTACHMENTS_PER_MESSAGE -
        attachmentsRef.current.length -
        pendingFileCountRef.current,
    )
    pendingFileCountRef.current += files.length
    setIsProcessingAttachments(true)

    const addFiles = pasteQueueRef.current.then(async () => {
      const added: SelectedAttachment[] = []
      for (const [index, file] of files.entries()) {
        const optimizedFile =
          index < availableOptimizationSlots &&
          file.size <= MAX_ATTACHMENT_BYTES
            ? await optimizeImageAttachment(file)
            : file
        added.push({
          id: crypto.randomUUID(),
          name: optimizedFile.name || 'Pasted file',
          size: optimizedFile.size,
          file: optimizedFile,
          previewUrl: PREVIEW_MIME_TYPES.has(optimizedFile.type)
            ? URL.createObjectURL(optimizedFile)
            : null,
        })
      }
      const nextAttachments = [...attachmentsRef.current, ...added]
      updateAttachments(nextAttachments)
      setError(null)
    })
    pasteQueueRef.current = addFiles.then(
      () => undefined,
      () => undefined,
    )
    const finishPaste = () => {
      pendingFileCountRef.current -= files.length
      setIsProcessingAttachments(pendingFileCountRef.current > 0)
    }
    void addFiles.then(finishPaste, finishPaste)
  }

  const removeAttachment = (id: string) => {
    const removed = attachmentsRef.current.find(
      (attachment) => attachment.id === id,
    )
    if (removed?.previewUrl !== null && removed?.previewUrl !== undefined) {
      URL.revokeObjectURL(removed.previewUrl)
    }
    updateAttachments(
      attachmentsRef.current.filter((attachment) => attachment.id !== id),
    )
  }

  const removeAttachments = (ids: readonly string[]) => {
    const removedIds = new Set(ids)
    const current = attachmentsRef.current
    current
      .filter((attachment) => removedIds.has(attachment.id))
      .forEach(({ previewUrl }) => {
        if (previewUrl !== null) URL.revokeObjectURL(previewUrl)
      })
    updateAttachments(
      current.filter((attachment) => !removedIds.has(attachment.id)),
    )
  }

  const sendMessage = async () => {
    if (attachmentLimitError(attachments) !== null) return
    if (
      (draft.trim() === '' && attachments.length === 0) ||
      !messageEnabled ||
      isSending ||
      pendingFileCountRef.current > 0 ||
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
      const succeeded = await onAction('messages', { content: sentDraft })
      if (succeeded) clearDraftIfUnchanged(sentDraft)
      finishSending()
      return
    }
    await ResultAsync.fromPromise(
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
    isProcessingAttachments,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  }
}
