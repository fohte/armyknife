import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { MessageComposer } from '#components/message-composer'

let container: HTMLDivElement
let root: Root | null = null

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function ComposerHarness({
  onSend = () => undefined,
  onPasteFiles = () => undefined,
  onRemoveAttachment = () => undefined,
}: {
  onSend?: () => void
  onPasteFiles?: (files: File[]) => void
  onRemoveAttachment?: (id: string) => void
}) {
  const [draft, setDraft] = useState('')
  return (
    <MessageComposer
      draft={draft}
      placeholder="Message session"
      disabled={false}
      isSending={false}
      isMobile={false}
      canInterrupt={false}
      attachments={[]}
      onDraftChange={setDraft}
      onKeyDown={() => undefined}
      onPasteFiles={onPasteFiles}
      onRemoveAttachment={onRemoveAttachment}
      onSend={onSend}
      onInterrupt={() => undefined}
    />
  )
}

function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, [value])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(<ComposerHarness />)
  })
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('grows with the draft and scrolls after reaching half the viewport', () => {
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  let scrollHeight = 120
  Object.defineProperty(textarea, 'scrollHeight', {
    configurable: true,
    get: () => scrollHeight,
  })
  setTextareaValue(textarea, 'A short message')

  const maxHeight = Number.parseFloat(
    window.getComputedStyle(textarea).maxHeight,
  )
  const snapshotHeight = () => ({
    maxHeight,
    height: Number.parseFloat(textarea.style.height),
    overflowY: window.getComputedStyle(textarea).overflowY,
  })
  const underCap = snapshotHeight()
  scrollHeight = 500
  setTextareaValue(textarea, 'A much longer message')
  const atCap = snapshotHeight()

  const snapshot = () => ({ underCap, atCap })
  expect(snapshot()).toEqual({
    underCap: {
      maxHeight,
      height: 120,
      overflowY: 'auto',
    },
    atCap: {
      maxHeight,
      height: maxHeight,
      overflowY: 'auto',
    },
  })
})

it('submits the message when the send button is clicked', () => {
  const onSend = vi.fn()
  act(() => {
    root?.render(<ComposerHarness onSend={onSend} />)
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Send from the button')
  let defaultPrevented = false
  const recordSubmit = (event: Event) => {
    defaultPrevented = event.defaultPrevented
  }
  window.addEventListener('submit', recordSubmit)
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('[aria-label="Send message"]'),
    ).click()
  })
  window.removeEventListener('submit', recordSubmit)

  const snapshot = () => ({
    defaultPrevented,
    sendCalls: onSend.mock.calls,
  })
  expect(snapshot()).toEqual({
    defaultPrevented: true,
    sendCalls: [[]],
  })
})

it('passes pasted files to the attachment handler and prevents text insertion', () => {
  const onPasteFiles = vi.fn()
  act(() => {
    root?.render(<ComposerHarness onPasteFiles={onPasteFiles} />)
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const file = new File(['image'], 'clipboard.png', { type: 'image/png' })
  const event = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(event)
  })

  const snapshot = () => ({
    defaultPrevented: event.defaultPrevented,
    files: onPasteFiles.mock.calls,
  })
  expect(snapshot()).toEqual({
    defaultPrevented: true,
    files: [[[file]]],
  })
})

it('removes an attachment when its remove button is clicked', () => {
  const onRemoveAttachment = vi.fn()
  act(() => {
    root?.render(
      <MessageComposer
        draft=""
        placeholder="Message session"
        disabled={false}
        isSending={false}
        isMobile={false}
        canInterrupt={false}
        attachments={[
          { id: 'attachment-1', name: 'clipboard.png', previewUrl: null },
        ]}
        onDraftChange={() => undefined}
        onKeyDown={() => undefined}
        onPasteFiles={() => undefined}
        onRemoveAttachment={onRemoveAttachment}
        onSend={() => undefined}
        onInterrupt={() => undefined}
      />,
    )
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Remove clipboard.png"]',
      ),
    ).click()
  })

  expect(onRemoveAttachment.mock.calls).toEqual([['attachment-1']])
})
