import { ResultAsync } from 'neverthrow'
import { useCallback, useRef, useState, useSyncExternalStore } from 'react'

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

type SessionAttachmentState = {
  attachments: SelectedAttachment[]
  pendingFileCount: number
  generation: number
}

const EMPTY_SESSION_ATTACHMENT_STATE: SessionAttachmentState = {
  attachments: [],
  pendingFileCount: 0,
  generation: 0,
}
const sessionAttachmentStates = new Map<string, SessionAttachmentState>()
const sessionAttachmentListeners = new Map<string, Set<() => void>>()

function getSessionAttachmentState(sessionId: string) {
  return (
    sessionAttachmentStates.get(sessionId) ?? EMPTY_SESSION_ATTACHMENT_STATE
  )
}

function notifySessionAttachmentListeners(sessionId: string) {
  sessionAttachmentListeners.get(sessionId)?.forEach((listener) => {
    listener()
  })
}

function updateSessionAttachmentState(
  sessionId: string,
  update: (current: SessionAttachmentState) => SessionAttachmentState,
) {
  const next = update(getSessionAttachmentState(sessionId))
  if (
    next.attachments.length === 0 &&
    next.pendingFileCount === 0 &&
    next.generation === 0
  ) {
    sessionAttachmentStates.delete(sessionId)
  } else {
    sessionAttachmentStates.set(sessionId, next)
  }
  notifySessionAttachmentListeners(sessionId)
}

function subscribeToSessionAttachments(
  sessionId: string,
  listener: () => void,
) {
  const listeners = sessionAttachmentListeners.get(sessionId) ?? new Set()
  listeners.add(listener)
  sessionAttachmentListeners.set(sessionId, listeners)
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) sessionAttachmentListeners.delete(sessionId)
  }
}

export function clearSessionAttachments(sessionId: string) {
  const current = getSessionAttachmentState(sessionId)
  current.attachments.forEach(({ previewUrl }) => {
    if (previewUrl !== null) URL.revokeObjectURL(previewUrl)
  })
  updateSessionAttachmentState(sessionId, (state) => ({
    ...state,
    attachments: [],
    pendingFileCount: 0,
    generation: state.generation + 1,
  }))
}

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
  const subscribe = useCallback(
    (listener: () => void) =>
      subscribeToSessionAttachments(sessionId, listener),
    [sessionId],
  )
  const getSnapshot = useCallback(
    () => getSessionAttachmentState(sessionId),
    [sessionId],
  )
  const sessionAttachmentState = useSyncExternalStore(
    subscribe,
    getSnapshot,
    getSnapshot,
  )
  const { attachments } = sessionAttachmentState
  const [sendPending, setSendPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const sendingRef = useRef(false)
  const pasteQueueRef = useRef(Promise.resolve())
  const isSending = externalSending || sendPending
  const limitError = attachmentLimitError(attachments)

  const updateAttachments = (nextAttachments: SelectedAttachment[]) => {
    updateSessionAttachmentState(sessionId, (state) => ({
      ...state,
      attachments: nextAttachments,
    }))
  }

  const onPasteFiles = (files: File[]) => {
    if (files.length === 0) return
    const currentState = getSessionAttachmentState(sessionId)
    const availableOptimizationSlots = Math.max(
      0,
      MAX_ATTACHMENTS_PER_MESSAGE -
        currentState.attachments.length -
        currentState.pendingFileCount,
    )
    const generation = currentState.generation
    updateSessionAttachmentState(sessionId, (state) => ({
      ...state,
      pendingFileCount: state.pendingFileCount + files.length,
    }))

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
      const current = getSessionAttachmentState(sessionId)
      if (current.generation !== generation) {
        added.forEach(({ previewUrl }) => {
          if (previewUrl !== null) URL.revokeObjectURL(previewUrl)
        })
        return
      }
      updateSessionAttachmentState(sessionId, (state) => ({
        ...state,
        attachments: [...state.attachments, ...added],
      }))
      setError(null)
    })
    pasteQueueRef.current = addFiles.then(
      () => undefined,
      () => undefined,
    )
    const finishPaste = () => {
      updateSessionAttachmentState(sessionId, (state) =>
        state.generation === generation
          ? {
              ...state,
              pendingFileCount: Math.max(
                0,
                state.pendingFileCount - files.length,
              ),
            }
          : state,
      )
    }
    void addFiles.then(finishPaste, finishPaste)
  }

  const removeAttachment = (id: string) => {
    const current = getSessionAttachmentState(sessionId).attachments
    const removed = current.find((attachment) => attachment.id === id)
    if (removed?.previewUrl !== null && removed?.previewUrl !== undefined) {
      URL.revokeObjectURL(removed.previewUrl)
    }
    updateAttachments(current.filter((attachment) => attachment.id !== id))
  }

  const removeAttachments = (ids: readonly string[]) => {
    const removedIds = new Set(ids)
    const current = getSessionAttachmentState(sessionId).attachments
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
      getSessionAttachmentState(sessionId).pendingFileCount > 0 ||
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
    isProcessingAttachments: sessionAttachmentState.pendingFileCount > 0,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  }
}
