import { act, type KeyboardEvent, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { MessageComposer } from '#components/message-composer'
import type { SkillSuggestion } from '#types'

let container: HTMLDivElement
let root: Root | null = null

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function ComposerHarness({
  isMobile = false,
  engine,
  skills = [],
  initialDraft = '',
  onSend = () => undefined,
  onKeyDown = () => undefined,
  onPasteFiles = () => undefined,
  onRemoveAttachment = () => undefined,
}: {
  isMobile?: boolean
  engine?: 'Claude' | 'Codex'
  skills?: readonly SkillSuggestion[]
  initialDraft?: string
  onSend?: () => void
  onKeyDown?: (event: KeyboardEvent<HTMLTextAreaElement>) => void
  onPasteFiles?: (files: File[]) => void
  onRemoveAttachment?: (id: string) => void
}) {
  const [draft, setDraft] = useState(initialDraft)
  return (
    <MessageComposer
      draft={draft}
      placeholder="Message session"
      disabled={false}
      isSending={false}
      isMobile={isMobile}
      engine={engine}
      skills={skills}
      canInterrupt={false}
      attachments={[]}
      onDraftChange={setDraft}
      onKeyDown={onKeyDown}
      onPasteFiles={onPasteFiles}
      onRemoveAttachment={onRemoveAttachment}
      onSend={onSend}
      onInterrupt={() => undefined}
    />
  )
}

function renderFreshComposer(props: Parameters<typeof ComposerHarness>[0]) {
  act(() => root?.unmount())
  root = createRoot(container)
  act(() => {
    root?.render(<ComposerHarness {...props} />)
  })
}

function setTextareaValue(
  textarea: HTMLTextAreaElement,
  value: string,
  cursor = value.length,
) {
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, [value])
    }
    textarea.setSelectionRange(cursor, cursor)
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

const skillSuggestions: SkillSuggestion[] = [
  { name: 'design-flow', description: 'Help shape a design.' },
  { name: 'debug-flow', description: 'Investigate and fix errors.' },
]

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it.each([
  { layout: 'desktop', isMobile: false, expectedRows: 2 },
  { layout: 'iPhone', isMobile: true, expectedRows: 1 },
])(
  '$layout composer starts with $expectedRows row(s)',
  ({ isMobile, expectedRows }) => {
    act(() => {
      root?.render(<ComposerHarness isMobile={isMobile} />)
    })

    const snapshot = () => ({
      rows: requireElement(
        container.querySelector<HTMLTextAreaElement>('textarea'),
      ).rows,
    })
    expect(snapshot()).toEqual({ rows: expectedRows })
  },
)

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
          {
            id: 'attachment-1',
            name: 'clipboard.png',
            size: 7,
            previewUrl: null,
          },
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

it('shows the size of a non-image attachment', () => {
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
          {
            id: 'attachment-1',
            name: 'design-mock.html',
            size: 12 * 1024 * 1024,
            previewUrl: null,
          },
        ]}
        onDraftChange={() => undefined}
        onKeyDown={() => undefined}
        onPasteFiles={() => undefined}
        onRemoveAttachment={() => undefined}
        onSend={() => undefined}
        onInterrupt={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    name: container.querySelector('.composer-attachment span')?.textContent,
    size: container.querySelector('.composer-attachment small')?.textContent,
  })
  expect(snapshot()).toEqual({
    name: 'design-mock.html',
    size: '12 MiB',
  })
})

it('shows the byte size of a small non-image attachment', () => {
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
          {
            id: 'attachment-1',
            name: 'sample.txt',
            size: 512,
            previewUrl: null,
          },
        ]}
        onDraftChange={() => undefined}
        onKeyDown={() => undefined}
        onPasteFiles={() => undefined}
        onRemoveAttachment={() => undefined}
        onSend={() => undefined}
        onInterrupt={() => undefined}
      />,
    )
  })

  const snapshot = () =>
    container.querySelector('.composer-attachment small')?.textContent
  expect(snapshot()).toEqual('512 B')
})

it('shows Claude skill suggestions after a slash following whitespace', () => {
  renderFreshComposer({
    engine: 'Claude',
    skills: skillSuggestions,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Explain /des')

  const snapshot = () => ({
    options: Array.from(container.querySelectorAll('[role="option"]')).map(
      (option) => option.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    options: ['/design-flowHelp shape a design.'],
  })
})

it('inserts the selected skill while preserving surrounding text', () => {
  renderFreshComposer({
    engine: 'Claude',
    isMobile: true,
    skills: skillSuggestions,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Explain /des today', 'Explain /des'.length)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('[role="option"]'),
    ).click()
  })

  const snapshot = () => ({
    draft: textarea.value,
    cursor: textarea.selectionStart,
    options: container.querySelectorAll('[role="option"]').length,
  })
  expect(snapshot()).toEqual({
    draft: 'Explain /design-flow today',
    cursor: 20,
    options: 0,
  })
})

it('selects a Codex skill with the dollar trigger by keyboard', () => {
  renderFreshComposer({
    engine: 'Codex',
    skills: skillSuggestions,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Check $de')
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowDown',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Enter',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    draft: textarea.value,
    options: container.querySelectorAll('[role="option"]').length,
  })
  expect(snapshot()).toEqual({
    draft: 'Check $debug-flow ',
    options: 0,
  })
})

it('selects the final skill with ArrowUp and Tab', () => {
  renderFreshComposer({
    engine: 'Codex',
    skills: skillSuggestions,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Check $de')
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowUp',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Tab',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    draft: textarea.value,
    options: container.querySelectorAll('[role="option"]').length,
  })
  expect(snapshot()).toEqual({
    draft: 'Check $debug-flow ',
    options: 0,
  })
})

it.each([
  {
    reason: 'Codex slash commands',
    engine: 'Codex' as const,
    skills: skillSuggestions,
    draft: 'Run /de',
  },
  {
    reason: 'a missing transcript catalog',
    engine: 'Claude' as const,
    skills: [],
    draft: 'Run /de',
  },
])('does not show suggestions for $reason', ({ engine, skills, draft }) => {
  renderFreshComposer({ engine, skills })
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    draft,
  )

  const snapshot = () => ({
    suggestions: container.querySelectorAll('[role="option"]').length,
  })
  expect(snapshot()).toEqual({ suggestions: 0 })
})

it('dismisses an open skill list with Escape', () => {
  renderFreshComposer({
    engine: 'Claude',
    skills: skillSuggestions,
  })
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    'Run /de',
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    visibleOptions: container.querySelectorAll('[role="option"]').length,
  })
  expect(snapshot()).toEqual({ visibleOptions: 0 })
})

it('does not restore an old cursor after selecting an unchanged skill', () => {
  renderFreshComposer({
    engine: 'Claude',
    skills: skillSuggestions,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, '/design-flow today', '/design-flow'.length)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('[role="option"]'),
    ).click()
  })
  setTextareaValue(textarea, '/design-flow! today', '/design-flow!'.length)

  const snapshot = () => ({
    draft: textarea.value,
    cursor: textarea.selectionStart,
  })
  expect(snapshot()).toEqual({
    draft: '/design-flow! today',
    cursor: '/design-flow!'.length,
  })
})

it('selects a matching skill after the caret moves to a narrower query', () => {
  const onKeyDown = vi.fn()
  renderFreshComposer({
    engine: 'Codex',
    skills: skillSuggestions,
    onKeyDown,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, '$d then $deb', '$d'.length)
  act(() => {
    textarea.setSelectionRange('$d'.length, '$d'.length)
    textarea.dispatchEvent(new Event('select', { bubbles: true }))
  })
  const initialOptions = container.querySelectorAll('[role="option"]').length
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowDown',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  act(() => {
    textarea.focus()
    textarea.setSelectionRange('$d then $deb'.length, '$d then $deb'.length)
    textarea.dispatchEvent(new Event('select', { bubbles: true }))
    document.dispatchEvent(new Event('selectionchange'))
  })
  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Enter',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    initialOptions,
    draft: textarea.value,
    delegatedKeydowns: onKeyDown.mock.calls.length,
    suggestions: container.querySelectorAll('[role="option"]').length,
  })
  expect(snapshot()).toEqual({
    initialOptions: 2,
    draft: '$d then $debug-flow ',
    delegatedKeydowns: 0,
    suggestions: 0,
  })
})
