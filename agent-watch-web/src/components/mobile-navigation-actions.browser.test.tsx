import '#index.css'

import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MobileNavigationActions } from '#components/mobile-navigation-actions'

let container: HTMLDivElement
let root: Root | null = null

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

it('opens the new session form from the Inbox header', async () => {
  const onNewSession = vi.fn()
  const args: ComponentProps<typeof MobileNavigationActions> = {
    onOpenNavigation: () => undefined,
    onOpenPalette: () => undefined,
    onNewSession,
  }
  root = createRoot(container)
  act(() => {
    root?.render(<MobileNavigationActions {...args} />)
  })
  const newSessionButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="New session"]',
  )
  if (newSessionButton === null)
    throw new Error('New session button is missing')

  await act(async () => {
    await userEvent.click(newSessionButton)
  })

  expect(onNewSession.mock.calls).toEqual([[]])
})
