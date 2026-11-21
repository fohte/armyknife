import { act, useRef, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { useFocusTrap } from '#components/use-focus-trap'

let container: HTMLDivElement
let root: Root | null = null

function FocusTrapHarness({
  initiallyOpen = false,
}: {
  initiallyOpen?: boolean
}) {
  const [open, setOpen] = useState(initiallyOpen)
  const containerRef = useRef<HTMLDivElement>(null)
  const initialFocusRef = useRef<HTMLButtonElement>(null)
  useFocusTrap({
    containerRef,
    initialFocusRef,
    isActive: open,
  })

  return (
    <>
      <button
        aria-label="Open dialog"
        type="button"
        onClick={() => {
          setOpen(true)
        }}
      >
        Open
      </button>
      {open ? (
        <div ref={containerRef} tabIndex={-1}>
          <button ref={initialFocusRef} type="button">
            First action
          </button>
          <button
            type="button"
            onClick={() => {
              setOpen(false)
            }}
          >
            Close dialog
          </button>
        </div>
      ) : null}
    </>
  )
}

function renderFocusTrap(initiallyOpen = false) {
  root = createRoot(container)
  act(() => {
    root?.render(<FocusTrapHarness initiallyOpen={initiallyOpen} />)
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('moves focus into the active container', () => {
  renderFocusTrap()
  const opener = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open dialog"]',
  )
  act(() => opener?.focus())
  act(() => opener?.click())
  const initialFocus = container.querySelector<HTMLButtonElement>(
    'button:not([aria-label])',
  )

  expect(document.activeElement).toBe(initialFocus)
})

it('restores focus to the opener when the active container closes', () => {
  renderFocusTrap()
  const opener = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open dialog"]',
  )
  act(() => opener?.focus())
  act(() => opener?.click())

  act(() => {
    container.querySelector<HTMLButtonElement>('button:last-child')?.click()
  })

  expect(document.activeElement).toBe(opener)
})

it('wraps Tab and Shift+Tab within the active container', () => {
  renderFocusTrap(true)
  const initialFocus = container.querySelector<HTMLButtonElement>(
    'button:not([aria-label])',
  )
  const lastAction =
    container.querySelector<HTMLButtonElement>('button:last-child')
  const focusSequence: string[] = []
  const recordFocus = () => {
    focusSequence.push(
      document.activeElement === initialFocus
        ? 'first'
        : document.activeElement === lastAction
          ? 'last'
          : 'outside',
    )
  }
  recordFocus()
  act(() => {
    document.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Tab',
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  recordFocus()
  act(() => {
    document.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Tab',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  recordFocus()

  expect(focusSequence).toEqual(['first', 'last', 'first'])
})
