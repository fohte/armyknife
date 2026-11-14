import '#index.css'

import { Button } from '@fohte/ui/button'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { BangCommandButton } from '#components/bang-command-button'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.className = 'agent-watch-shell'
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('keeps plain button hover neutral while destructive hover stays red', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <>
        <Button aria-label="neutral action" variant="plain">
          More
        </Button>
        <Button aria-label="destructive action" variant="destructive">
          Reject
        </Button>
      </>,
    )
  })

  const plainButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="neutral action"]',
  )
  const destructiveButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="destructive action"]',
  )
  if (plainButton === null || destructiveButton === null) {
    throw new Error('Expected both buttons to render')
  }

  await userEvent.hover(plainButton)
  const plainHoverColor = getComputedStyle(plainButton).color
  await userEvent.hover(destructiveButton)
  await new Promise((resolve) => setTimeout(resolve, 250))

  const getHoverColors = () => ({
    plainHoverColor,
    destructiveHoverColor: getComputedStyle(destructiveButton).color,
  })
  expect(getHoverColors()).toEqual({
    plainHoverColor: 'rgb(212, 212, 216)',
    destructiveHoverColor: 'rgb(248, 113, 113)',
  })
})

it('shows completed command status in the success theme color', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <BangCommandButton
        label="Run command"
        message="Completed"
        onClick={() => {}}
        status="succeeded"
      />,
    )
  })

  const status = container.querySelector<HTMLElement>('[role="status"]')
  if (status === null) throw new Error('Expected the command status to render')

  expect(getComputedStyle(status).color).toBe('rgb(74, 222, 128)')
})
