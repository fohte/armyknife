import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

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

it('renders mobile confirmation details inside a bottom sheet', () => {
  act(() => {
    root?.render(
      <ConfirmDialog
        title="Confirm the merge?"
        confirmLabel="Confirm merge"
        mobile
        portal={false}
        onCancel={() => undefined}
        onConfirm={() => undefined}
      >
        <code className="pull-request-merge-head-sha">fixture-head-sha</code>
        <dl className="bang-command-dialog-details">
          <dt>Working directory</dt>
          <dd>
            <code>/workspace/example</code>
          </dd>
          <dt>Command</dt>
          <dd>
            <pre>git status --short</pre>
          </dd>
        </dl>
      </ConfirmDialog>,
    )
  })

  const snapshot = () => ({
    backdrop: container.querySelector('.confirm-dialog-backdrop')?.className,
    head: container.querySelector('.pull-request-merge-head-sha')?.textContent,
    details: Array.from(
      container.querySelectorAll(
        '.bang-command-dialog-details dt, .bang-command-dialog-details dd',
      ),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    backdrop: 'confirm-dialog-backdrop confirm-dialog-backdrop--mobile',
    head: 'fixture-head-sha',
    details: [
      'Working directory',
      '/workspace/example',
      'Command',
      'git status --short',
    ],
  })
})

it('activates the cancel action when Enter is pressed on the cancel button', async () => {
  const onCancel = vi.fn()
  const onConfirm = vi.fn()
  act(() => {
    root?.render(
      <ConfirmDialog
        title="Confirm action?"
        confirmLabel="Confirm"
        onCancel={onCancel}
        onConfirm={onConfirm}
      />,
    )
  })

  const cancelButton = Array.from(document.querySelectorAll('button')).find(
    (button) => button.textContent === 'Cancel',
  )
  if (cancelButton === undefined) throw new Error('Expected Cancel button')

  await act(async () => {
    cancelButton.focus()
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    cancelCalls: onCancel.mock.calls.length,
    confirmCalls: onConfirm.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    cancelCalls: 1,
    confirmCalls: 0,
  })
})
