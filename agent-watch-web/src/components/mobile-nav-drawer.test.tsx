import { act, type ReactNode, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MobileNavDrawer } from '#components/mobile-nav-drawer'

let container: HTMLDivElement
let root: Root | null = null

function ControlledMobileNav({
  children,
  onClose,
}: {
  children: ReactNode
  onClose: () => void
}) {
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
        {children}
      </MobileNavDrawer>
    </>
  )
}

function renderMobileNav(
  onClose: () => void,
  children: ReactNode = <button type="button">Navigation item</button>,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <ControlledMobileNav onClose={onClose}>{children}</ControlledMobileNav>,
    )
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
  const modal = dialog.matches(':modal')
  await act(async () => {
    await userEvent.keyboard('{Escape}')
  })

  const snapshot = () => ({
    opened,
    modal,
    closed: dialog.open,
    onCloseCalls: onClose.mock.calls,
  })
  expect(snapshot()).toEqual({
    opened: true,
    modal: true,
    closed: false,
    onCloseCalls: [[]],
  })
})

it('fills the viewport and scrolls long navigation inside the drawer', async () => {
  const dialog = renderMobileNav(
    vi.fn(),
    <div>
      {Array.from({ length: 80 }, (_, index) => (
        <div key={index}>Navigation item {index + 1}</div>
      ))}
    </div>,
  )
  const trigger = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open navigation"]',
  )
  if (trigger === null)
    throw new Error('Mobile navigation trigger was not rendered')

  await act(async () => {
    await userEvent.click(trigger)
  })

  dialog.style.setProperty('--mobile-viewport-top', '120px')
  dialog.style.setProperty('--mobile-viewport-height', '420px')
  const viewportHeight = 420
  const snapshot = () => ({
    drawerTop: getComputedStyle(dialog).top,
    drawerHeight: Math.round(dialog.getBoundingClientRect().height),
    viewportHeight,
    overflowY: getComputedStyle(dialog).overflowY,
    contentOverflows: dialog.scrollHeight > dialog.clientHeight,
  })
  expect(snapshot()).toEqual({
    drawerTop: '120px',
    drawerHeight: viewportHeight,
    viewportHeight,
    overflowY: 'auto',
    contentOverflows: true,
  })
})
