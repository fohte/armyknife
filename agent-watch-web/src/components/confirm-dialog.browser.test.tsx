import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConfirmDialog } from '#components/confirm-dialog'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('keeps a pending dialog open and stops Escape from reaching global shortcuts', () => {
  const onCancel = vi.fn()
  const onConfirm = vi.fn()
  const globalKeyDown = vi.fn()
  window.addEventListener('keydown', globalKeyDown)
  act(() => {
    root?.render(
      <ConfirmDialog
        title="Confirm action?"
        confirmLabel="Confirm"
        pending
        onCancel={onCancel}
        onConfirm={onConfirm}
      />,
    )
  })

  act(() => {
    document
      .querySelector<HTMLButtonElement>('.confirm-dialog-confirm')
      ?.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'Escape',
          bubbles: true,
          cancelable: true,
        }),
      )
  })
  window.removeEventListener('keydown', globalKeyDown)

  const snapshot = () => ({
    cancelCalls: onCancel.mock.calls.length,
    confirmCalls: onConfirm.mock.calls.length,
    globalKeyDownCalls: globalKeyDown.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    cancelCalls: 0,
    confirmCalls: 0,
    globalKeyDownCalls: 0,
  })
})
