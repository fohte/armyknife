import '#index.css'

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MobileSheet } from '#components/mobile-sheet'

let container: HTMLDivElement
let root: Root | null = null

function SheetHarness({
  onClose,
  presentation,
  canDismiss,
}: {
  onClose: () => void
  presentation?: 'bottom' | 'fullscreen'
  canDismiss?: boolean
}) {
  const [open, setOpen] = useState(false)

  return (
    <>
      <button
        aria-label="Open sheet"
        type="button"
        onClick={() => {
          setOpen(true)
        }}
      >
        Open
      </button>
      {open ? (
        <MobileSheet
          {...(canDismiss === undefined ? {} : { canDismiss })}
          label="Example sheet"
          onClose={() => {
            onClose()
            setOpen(false)
          }}
          {...(presentation === undefined ? {} : { presentation })}
        >
          <button type="button">Inside action</button>
        </MobileSheet>
      ) : null}
    </>
  )
}

async function openSheet(props: {
  onClose: () => void
  presentation?: 'bottom' | 'fullscreen'
  canDismiss?: boolean
}) {
  root = createRoot(container)
  act(() => {
    root?.render(<SheetHarness {...props} />)
  })
  const opener = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open sheet"]',
  )
  if (opener === null) throw new Error('The sheet opener is missing')
  await act(async () => {
    await userEvent.click(opener)
  })
  return opener
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
  document.documentElement.style.removeProperty('--mobile-viewport-top')
  document.documentElement.style.removeProperty('--mobile-viewport-height')
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('traps focus in a dialog and restores it after Escape closes the sheet', async () => {
  const onClose = vi.fn()
  const opener = await openSheet({ onClose })
  const dialog = container.querySelector<HTMLElement>('[role="dialog"]')
  const insideAction = container.querySelector<HTMLButtonElement>(
    'button:not([aria-label])',
  )

  const openSnapshot = () => ({
    label: dialog?.getAttribute('aria-label') ?? null,
    modal: dialog?.getAttribute('aria-modal') ?? null,
    role: dialog?.getAttribute('role') ?? null,
    focusIsInside: dialog?.contains(document.activeElement) ?? false,
  })
  expect(openSnapshot()).toEqual({
    label: 'Example sheet',
    modal: 'true',
    role: 'dialog',
    focusIsInside: true,
  })

  if (insideAction === null) throw new Error('The sheet action is missing')
  await act(async () => {
    insideAction.focus()
    await userEvent.keyboard('{Escape}')
  })

  const closedSnapshot = () => ({
    closeCalls: onClose.mock.calls,
    dialogIsOpen: container.querySelector('[role="dialog"]') !== null,
    focusReturnedToOpener: document.activeElement === opener,
  })
  expect(closedSnapshot()).toEqual({
    closeCalls: [[]],
    dialogIsOpen: false,
    focusReturnedToOpener: true,
  })
})

it('closes on backdrop clicks without closing when content is clicked', async () => {
  const onClose = vi.fn()
  await openSheet({ onClose })
  const insideAction = container.querySelector<HTMLButtonElement>(
    'button:not([aria-label])',
  )
  const overlay = container.querySelector<HTMLElement>('.mobile-sheet-overlay')
  if (insideAction === null || overlay === null) {
    throw new Error('The sheet controls are missing')
  }
  const snapshot = () => ({
    closeCalls: onClose.mock.calls,
    dialogIsOpen: container.querySelector('[role="dialog"]') !== null,
  })

  await act(async () => {
    await userEvent.click(insideAction)
  })

  expect(snapshot()).toEqual({
    closeCalls: [],
    dialogIsOpen: true,
  })

  await act(async () => {
    await userEvent.click(overlay)
  })

  expect(snapshot()).toEqual({
    closeCalls: [[]],
    dialogIsOpen: false,
  })
})

it('closes when a pointerdown lands outside the overlay', async () => {
  const onClose = vi.fn()
  await openSheet({ onClose })
  const outside = document.createElement('button')
  outside.type = 'button'
  container.append(outside)

  act(() => {
    outside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })

  const snapshot = () => ({
    closeCalls: onClose.mock.calls,
    dialogIsOpen: container.querySelector('[role="dialog"]') !== null,
  })
  expect(snapshot()).toEqual({
    closeCalls: [[]],
    dialogIsOpen: false,
  })
})

it('keeps a non-dismissible full-screen sheet open', async () => {
  const onClose = vi.fn()
  await openSheet({
    canDismiss: false,
    onClose,
    presentation: 'fullscreen',
  })
  const overlay = container.querySelector<HTMLElement>('.mobile-sheet-overlay')

  if (overlay === null) throw new Error('The sheet overlay is missing')
  await act(async () => {
    await userEvent.click(overlay)
    await userEvent.keyboard('{Escape}')
  })

  const dialog = container.querySelector<HTMLElement>('[role="dialog"]')
  const snapshot = () => ({
    closeCalls: onClose.mock.calls,
    dialogClass: dialog?.className ?? null,
    presentation: overlay.getAttribute('data-presentation'),
  })
  expect(snapshot()).toEqual({
    closeCalls: [],
    dialogClass: 'mobile-sheet mobile-sheet--fullscreen',
    presentation: 'fullscreen',
  })
})

it('fits a full-screen sheet to the visual viewport', async () => {
  document.documentElement.style.setProperty('--mobile-viewport-top', '40px')
  document.documentElement.style.setProperty(
    '--mobile-viewport-height',
    '320px',
  )
  await openSheet({ onClose: vi.fn(), presentation: 'fullscreen' })
  const overlay = container.querySelector<HTMLElement>('.mobile-sheet-overlay')
  const sheet = container.querySelector<HTMLElement>('.mobile-sheet')
  if (overlay === null || sheet === null) {
    throw new Error('The full-screen sheet is missing')
  }

  const snapshot = () => ({
    overlayTop: overlay.getBoundingClientRect().top,
    overlayHeight: overlay.getBoundingClientRect().height,
    sheetTop: sheet.getBoundingClientRect().top,
    sheetBottom: sheet.getBoundingClientRect().bottom,
  })
  expect(snapshot()).toEqual({
    overlayTop: 40,
    overlayHeight: 320,
    sheetTop: 40,
    sheetBottom: 360,
  })
})
