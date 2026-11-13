import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { useComposerMessage } from '#components/use-composer-message'

vi.mock('#lib/image-attachment', () => ({
  optimizeImageAttachment: (file: File) => Promise.resolve(file),
}))

let container: HTMLDivElement
let root: Root | null = null
let composerMessage: ReturnType<typeof useComposerMessage> | null = null
let revokedUrls: string[] = []

function result() {
  if (composerMessage === null) throw new Error('Composer hook is not mounted')
  return composerMessage
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
  composerMessage = useComposerMessage({
    sessionId,
    draft: '',
    messageEnabled: true,
    externalSending: false,
    clearDraftIfUnchanged: () => undefined,
    onAction,
  })
  return null
}

function renderSession(sessionId: string) {
  act(() => {
    root?.render(
      <ComposerMessageHarness key={sessionId} sessionId={sessionId} />,
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
