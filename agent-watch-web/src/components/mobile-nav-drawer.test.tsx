import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MobileNavDrawer } from '#components/mobile-nav-drawer'

let container: HTMLDivElement
let root: Root | null = null

function ControlledMobileNav({ onClose }: { onClose: () => void }) {
  const [open, setOpen] = useState(false)

  return (
    <>
      <button
        aria-label="Open navigation"
        onClick={() => {
          setOpen(true)
        }}
        type="button"
      >
        Open
      </button>
      <MobileNavDrawer
        open={open}
        onClose={() => {
          onClose()
          setOpen(false)
        }}
      >
        <button type="button">Navigation item</button>
      </MobileNavDrawer>
    </>
  )
}

function renderMobileNav(onClose: () => void) {
  root = createRoot(container)
  act(() => {
    root?.render(<ControlledMobileNav onClose={onClose} />)
  })

  const dialog =
    container.querySelector<HTMLDialogElement>('.mobile-nav-drawer')
  if (dialog === null) throw new Error('Mobile navigation was not rendered')
  return dialog
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

it('opens the modal and calls onClose when Escape closes it', async () => {
  const onClose = vi.fn()
  const dialog = renderMobileNav(onClose)
  const trigger = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open navigation"]',
  )
  if (trigger === null)
    throw new Error('Mobile navigation trigger was not rendered')

  await act(async () => {
    await userEvent.click(trigger)
  })
  const opened = dialog.open
  await act(async () => {
    await userEvent.keyboard('{Escape}')
  })

  const snapshot = () => ({
    opened,
    closed: dialog.open,
    onCloseCalls: onClose.mock.calls,
  })
  expect(snapshot()).toEqual({
    opened: true,
    closed: false,
    onCloseCalls: [[]],
  })
})
