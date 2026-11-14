import { ResultAsync } from 'neverthrow'
import { useCallback, useState, useSyncExternalStore } from 'react'

import type { AttachmentPreview } from '#components/attachment-preview-list'
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

type SelectedAttachment = AttachmentPreview & { file: File }

type AttachmentState = {
  attachments: SelectedAttachment[]
  pendingFileCount: number
  generation: number
}

export type EncodedAttachment = {
  name: string
  mimeType: string
  size: number
  data: string
}

const EMPTY_STATE: AttachmentState = {
  attachments: [],
  pendingFileCount: 0,
  generation: 0,
}
const attachmentStates = new Map<string, AttachmentState>()
const attachmentListeners = new Map<string, Set<() => void>>()
const pasteQueues = new Map<string, Promise<void>>()

function getState(scopeId: string) {
  return attachmentStates.get(scopeId) ?? EMPTY_STATE
}

function notify(scopeId: string) {
  attachmentListeners.get(scopeId)?.forEach((listener) => {
    listener()
  })
}

function updateState(
  scopeId: string,
  update: (current: AttachmentState) => AttachmentState,
) {
  const next = update(getState(scopeId))
  if (
    next.attachments.length === 0 &&
    next.pendingFileCount === 0 &&
    next.generation === 0
  ) {
    attachmentStates.delete(scopeId)
  } else {
    attachmentStates.set(scopeId, next)
  }
  notify(scopeId)
}

function subscribe(scopeId: string, listener: () => void) {
  const listeners = attachmentListeners.get(scopeId) ?? new Set()
  listeners.add(listener)
  attachmentListeners.set(scopeId, listeners)
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) attachmentListeners.delete(scopeId)
  }
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

async function encodeAttachment(file: File): Promise<EncodedAttachment> {
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

export function clearSessionAttachments(sessionId: string) {
  for (const scopeId of attachmentStates.keys()) {
    if (scopeId !== sessionId && !scopeId.startsWith(`${sessionId}:`)) continue
    const current = getState(scopeId)
    current.attachments.forEach(({ previewUrl }) => {
      if (previewUrl !== null) URL.revokeObjectURL(previewUrl)
    })
    updateState(scopeId, (state) => ({
      ...state,
      attachments: [],
      pendingFileCount: 0,
      generation: state.generation + 1,
    }))
  }
}

export function useAttachments(scopeId: string) {
  const subscribeToScope = useCallback(
    (listener: () => void) => subscribe(scopeId, listener),
    [scopeId],
  )
  const getSnapshot = useCallback(() => getState(scopeId), [scopeId])
  const state = useSyncExternalStore(subscribeToScope, getSnapshot, getSnapshot)
  const [error, setError] = useState<string | null>(null)
  const limitError = attachmentLimitError(state.attachments)

  const onPasteFiles = (files: File[]) => {
    if (files.length === 0) return
    const currentState = getState(scopeId)
    const availableOptimizationSlots = Math.max(
      0,
      MAX_ATTACHMENTS_PER_MESSAGE -
        currentState.attachments.length -
        currentState.pendingFileCount,
    )
    const generation = currentState.generation
    updateState(scopeId, (current) => ({
      ...current,
      pendingFileCount: current.pendingFileCount + files.length,
    }))

    const addFiles = (pasteQueues.get(scopeId) ?? Promise.resolve()).then(
      async () => {
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
        const current = getState(scopeId)
        if (current.generation !== generation) {
          added.forEach(({ previewUrl }) => {
            if (previewUrl !== null) URL.revokeObjectURL(previewUrl)
          })
          return
        }
        updateState(scopeId, (currentState) => ({
          ...currentState,
          attachments: [...currentState.attachments, ...added],
        }))
        setError(null)
      },
    )
    const pasteQueue = addFiles.then(
      () => undefined,
      () => undefined,
    )
    pasteQueues.set(scopeId, pasteQueue)
    void pasteQueue.then(() => {
      if (pasteQueues.get(scopeId) === pasteQueue) pasteQueues.delete(scopeId)
    })
    const finishPaste = () => {
      updateState(scopeId, (current) =>
        current.generation === generation
          ? {
              ...current,
              pendingFileCount: Math.max(
                0,
                current.pendingFileCount - files.length,
              ),
            }
          : current,
      )
    }
    void addFiles.then(finishPaste, finishPaste)
  }

  const removeAttachments = (ids: readonly string[]) => {
    const removedIds = new Set(ids)
    const current = getState(scopeId).attachments
    current
      .filter((attachment) => removedIds.has(attachment.id))
      .forEach(({ previewUrl }) => {
        if (previewUrl !== null) URL.revokeObjectURL(previewUrl)
      })
    updateState(scopeId, (currentState) => ({
      ...currentState,
      attachments: currentState.attachments.filter(
        (attachment) => !removedIds.has(attachment.id),
      ),
    }))
  }

  const removeAttachment = (id: string) => {
    removeAttachments([id])
  }

  const encodeAttachments = () =>
    ResultAsync.fromPromise(
      Promise.all(state.attachments.map(({ file }) => encodeAttachment(file))),
      toError,
    )

  const sendWithAttachments = async (
    send: (attachments: readonly EncodedAttachment[]) => Promise<boolean>,
  ) => {
    const succeeded =
      state.attachments.length === 0
        ? await send([])
        : await encodeAttachments().match(send, (requestError) => {
            setError(requestError.message)
            return false
          })
    if (succeeded) setError(null)
    return succeeded
  }

  return {
    attachments: state.attachments,
    error: limitError ?? error,
    hasLimitError: limitError !== null,
    isProcessingAttachments: state.pendingFileCount > 0,
    onPasteFiles,
    removeAttachment,
    removeAttachments,
    sendWithAttachments,
  }
}
