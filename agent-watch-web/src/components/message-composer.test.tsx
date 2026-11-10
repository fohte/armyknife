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

function ComposerHarness() {
  const [draft, setDraft] = useState('')
  return (
    <MessageComposer
      draft={draft}
      placeholder="Message session"
      disabled={false}
      isSending={false}
      isMobile={false}
      canInterrupt={false}
      onDraftChange={setDraft}
      onKeyDown={() => undefined}
      onSend={() => undefined}
      onInterrupt={() => undefined}
    />
  )
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
  Object.defineProperty(textarea, 'scrollHeight', {
    configurable: true,
    get: () => 500,
  })
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, ['A long message'])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })

  const maxHeight = Number.parseFloat(
    window.getComputedStyle(textarea).maxHeight,
  )
  const snapshot = () => ({
    maxHeight,
    height: Number.parseFloat(textarea.style.height),
    overflowY: textarea.style.overflowY,
  })
  expect(snapshot()).toEqual({
    maxHeight,
    height: maxHeight,
    overflowY: 'auto',
  })
})
