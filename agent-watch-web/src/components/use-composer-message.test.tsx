import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { useComposerDraft } from '#components/use-composer-draft'
import {
  clearSessionAttachments,
  useComposerMessage,
} from '#components/use-composer-message'

const { optimizeImageAttachmentMock } = vi.hoisted(() => ({
  optimizeImageAttachmentMock: vi.fn<(file: File) => Promise<File>>(),
}))

vi.mock('#lib/image-attachment', () => ({
  optimizeImageAttachment: optimizeImageAttachmentMock,
}))

let container: HTMLDivElement
let root: Root | null = null
let composerMessage: ReturnType<typeof useComposerMessage> | null = null
let composerDraft: ReturnType<typeof useComposerDraft> | null = null
let revokedUrls: string[] = []

function result() {
  if (composerMessage === null) throw new Error('Composer hook is not mounted')
  return composerMessage
}

function currentDraft() {
  if (composerDraft === null) throw new Error('Composer draft is not mounted')
  return composerDraft
}

function ComposerMessageHarness({
  sessionId,
  onAction = () => Promise.resolve(true),
}: {
  sessionId: string
  onAction?: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
}) {
  composerDraft = useComposerDraft(sessionId)
  composerMessage = useComposerMessage({
    sessionId,
    draft: composerDraft.draft,
    messageEnabled: true,
    externalSending: false,
    clearDraftForSend: composerDraft.clearDraftForSend,
    settleDraftSend: composerDraft.settleDraftSend,
    onAction,
  })
  return null
}

function renderSession(
  sessionId: string,
  onAction?: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>,
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <ComposerMessageHarness
        key={sessionId}
        sessionId={sessionId}
        {...(onAction === undefined ? {} : { onAction })}
      />,
    )
  })
}

async function pasteImage(name: string) {
  await act(async () => {
    result().onPasteFiles([
      new File(['image data'], name, { type: 'image/png' }),
    ])
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

function attachmentSnapshot() {
  return result().attachments.map(({ id, name, previewUrl, file }) => ({
    id,
    name,
    previewUrl,
    file: { name: file.name, type: file.type, size: file.size },
  }))
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  composerMessage = null
  composerDraft = null
  optimizeImageAttachmentMock.mockReset()
  optimizeImageAttachmentMock.mockImplementation((file) =>
    Promise.resolve(file),
  )
  revokedUrls = []
  let previewId = 0
  vi.spyOn(URL, 'createObjectURL').mockImplementation(
    () => `blob:preview-${String(++previewId)}`,
  )
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation((url) => {
    revokedUrls.push(url)
  })
  renderSession(crypto.randomUUID())
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  composerMessage = null
  container.remove()
  window.localStorage.clear()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('restores attachments after switching away from and back to a session', async () => {
  const firstSession = crypto.randomUUID()
  const secondSession = crypto.randomUUID()
  renderSession(firstSession)
  await pasteImage('first.png')
  const firstSessionAttachments = attachmentSnapshot()

  renderSession(secondSession)
  renderSession(firstSession)

  const snapshot = () => ({
    attachments: attachmentSnapshot(),
    revokedUrls,
  })
  expect(snapshot()).toEqual({
    attachments: firstSessionAttachments,
    revokedUrls: [],
  })
})

it('keeps each session attachments separate', async () => {
  const firstSession = crypto.randomUUID()
  const secondSession = crypto.randomUUID()
  renderSession(firstSession)
  await pasteImage('first.png')

  renderSession(secondSession)
  const secondSessionAttachmentsBeforePaste = attachmentSnapshot()
  await pasteImage('second.png')
  const secondSessionAttachments = attachmentSnapshot()

  renderSession(firstSession)

  const snapshot = () => ({
    secondSessionAttachmentsBeforePaste,
    secondSessionAttachmentNames: secondSessionAttachments.map(
      ({ name }) => name,
    ),
    restoredFirstSessionAttachmentNames: attachmentSnapshot().map(
      ({ name }) => name,
    ),
  })
  expect(snapshot()).toEqual({
    secondSessionAttachmentsBeforePaste: [],
    secondSessionAttachmentNames: ['second.png'],
    restoredFirstSessionAttachmentNames: ['first.png'],
  })
})

it('clears pending attachments and revokes preview URLs when a session closes', async () => {
  const sessionId = crypto.randomUUID()
  renderSession(sessionId)
  await pasteImage('close-me.png')
  const previewUrl = attachmentSnapshot()[0]?.previewUrl

  act(() => {
    clearSessionAttachments(sessionId)
  })

  const snapshot = () => ({ attachments: attachmentSnapshot(), revokedUrls })
  expect(snapshot()).toEqual({
    attachments: [],
    revokedUrls: [previewUrl],
  })
})

it('restores an attachment when optimization finishes after returning to its session', async () => {
  const firstSession = crypto.randomUUID()
  const secondSession = crypto.randomUUID()
  const onAction = vi
    .fn<
      (action: string, payload?: Record<string, unknown>) => Promise<boolean>
    >()
    .mockResolvedValue(true)
  const file = new File(['image data'], 'pending.png', { type: 'image/png' })
  let finishOptimization: ((file: File) => void) | undefined
  optimizeImageAttachmentMock.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOptimization = resolve
      }),
  )
  renderSession(firstSession)
  act(() => {
    result().onPasteFiles([file])
  })
  await vi.waitFor(() => {
    if (optimizeImageAttachmentMock.mock.calls.length !== 1) {
      throw new Error('Image optimization has not started')
    }
  })

  renderSession(secondSession)
  renderSession(firstSession, onAction)
  const processingAfterReturn = result().isProcessingAttachments
  const attachmentNamesBeforeOptimization = result().attachments.map(
    ({ name }) => name,
  )

  const complete = finishOptimization
  if (complete === undefined) throw new Error('Expected optimization to start')
  await act(async () => {
    complete(file)
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  const restoredAttachmentNames = result().attachments.map(({ name }) => name)
  const attachmentPreviewUrl = result().attachments[0]?.previewUrl

  await act(async () => {
    await result().sendMessage()
  })

  const snapshot = () => ({
    processingAfterReturn,
    attachmentNamesBeforeOptimization,
    restoredAttachmentNames,
    actionCalls: onAction.mock.calls,
    revokedUrls,
    attachmentPreviewUrl,
  })
  expect(snapshot()).toEqual({
    processingAfterReturn: true,
    attachmentNamesBeforeOptimization: [],
    restoredAttachmentNames: ['pending.png'],
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'pending.png',
              mimeType: 'image/png',
              size: 10,
              data: 'aW1hZ2UgZGF0YQ==',
            },
          ],
        },
      ],
    ],
    revokedUrls: [attachmentPreviewUrl],
    attachmentPreviewUrl: 'blob:preview-1',
  })
})

it('revokes a preview URL when its attachment is removed', async () => {
  await pasteImage('removed.png')
  const [attachment] = result().attachments
  if (attachment === undefined) throw new Error('Expected an attachment')

  act(() => {
    result().removeAttachment(attachment.id)
  })

  const snapshot = () => ({
    attachments: attachmentSnapshot(),
    revokedUrls,
  })
  expect(snapshot()).toEqual({
    attachments: [],
    revokedUrls: [attachment.previewUrl],
  })
})

it('preserves a newer draft when an older send fails after remount', async () => {
  const sessionId = crypto.randomUUID()
  let finishSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        finishSend = resolve
      }),
  )
  renderSession(sessionId, onAction)
  act(() => {
    currentDraft().updateDraft('Original request')
  })
  let pendingSend: Promise<void> | undefined
  act(() => {
    pendingSend = result().sendMessage()
  })
  act(() => {
    root?.unmount()
  })
  root = null
  renderSession(sessionId, onAction)
  act(() => {
    currentDraft().updateDraft('New request')
  })

  if (pendingSend === undefined) throw new Error('Expected a pending send')
  await act(async () => {
    finishSend?.(false)
    await pendingSend
  })

  const snapshot = () => ({
    visibleDraft: currentDraft().draft,
    storedDraft: window.localStorage.getItem(
      `agent-watch-web:composer-draft:${sessionId}`,
    ),
  })
  expect(snapshot()).toEqual({
    visibleDraft: 'New request',
    storedDraft: 'New request',
  })
})

it('keeps the sent draft recoverable after remounting during a pending send', async () => {
  const sessionId = crypto.randomUUID()
  let finishSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        finishSend = resolve
      }),
  )
  renderSession(sessionId, onAction)
  act(() => {
    currentDraft().updateDraft('Recover this request')
  })
  let pendingSend: Promise<void> | undefined
  act(() => {
    pendingSend = result().sendMessage()
  })

  act(() => {
    root?.unmount()
  })
  root = null
  renderSession(sessionId, onAction)

  const snapshot = () => ({
    visibleDraft: currentDraft().draft,
    storedDraft: window.localStorage.getItem(
      `agent-watch-web:composer-draft:${sessionId}`,
    ),
  })
  expect(snapshot()).toEqual({
    visibleDraft: '',
    storedDraft: 'Recover this request',
  })

  if (pendingSend === undefined) throw new Error('Expected a pending send')
  await act(async () => {
    finishSend?.(false)
    await pendingSend
  })

  const restoredSnapshot = () => ({
    visibleDraft: currentDraft().draft,
    storedDraft: window.localStorage.getItem(
      `agent-watch-web:composer-draft:${sessionId}`,
    ),
  })
  expect(restoredSnapshot()).toEqual({
    visibleDraft: 'Recover this request',
    storedDraft: 'Recover this request',
  })
})

it('clears the sent draft from a remounted composer after success', async () => {
  const sessionId = crypto.randomUUID()
  let finishSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        finishSend = resolve
      }),
  )
  renderSession(sessionId, onAction)
  act(() => {
    currentDraft().updateDraft('Sent request')
  })
  let pendingSend: Promise<void> | undefined
  act(() => {
    pendingSend = result().sendMessage()
  })

  act(() => {
    root?.unmount()
  })
  root = null
  renderSession(sessionId, onAction)

  if (pendingSend === undefined) throw new Error('Expected a pending send')
  await act(async () => {
    finishSend?.(true)
    await pendingSend
  })

  const snapshot = () => ({
    visibleDraft: currentDraft().draft,
    storedDraft: window.localStorage.getItem(
      `agent-watch-web:composer-draft:${sessionId}`,
    ),
  })
  expect(snapshot()).toEqual({
    visibleDraft: '',
    storedDraft: null,
  })
})

it('revokes preview URLs after a message with attachments is sent', async () => {
  const onAction = vi
    .fn<(action: string) => Promise<boolean>>()
    .mockResolvedValue(true)
  const sessionId = crypto.randomUUID()
  act(() => {
    root?.render(
      <ComposerMessageHarness
        key={sessionId}
        sessionId={sessionId}
        onAction={onAction}
      />,
    )
  })
  await pasteImage('sent.png')
  const [attachment] = result().attachments
  if (attachment === undefined) throw new Error('Expected an attachment')

  await act(async () => {
    await result().sendMessage()
  })

  const snapshot = () => ({
    attachments: attachmentSnapshot(),
    revokedUrls,
    action: onAction.mock.calls[0]?.[0],
    error: result().error,
    isProcessingAttachments: result().isProcessingAttachments,
    isSending: result().isSending,
  })
  expect(snapshot()).toEqual({
    attachments: [],
    revokedUrls: [attachment.previewUrl],
    action: 'messages',
    error: null,
    isProcessingAttachments: false,
    isSending: false,
  })
})
