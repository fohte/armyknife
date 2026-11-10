import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { CritPane } from '#components/crit-pane'

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
  vi.unstubAllGlobals()
})

it('toggles the review between its column and fullscreen layouts', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <CritPane
        url="http://127.0.0.1:5000/review/sample"
        onClose={() => undefined}
      />,
    )
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Show crit fullscreen"]',
      )
      ?.click()
  })
  const fullscreenState = {
    className: container.querySelector('aside')?.className,
    exitLabel: container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Exit crit fullscreen"]',
      )
      ?.getAttribute('aria-pressed'),
  }

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Exit crit fullscreen"]',
      )
      ?.click()
  })

  const snapshot = () => ({
    fullscreenState,
    restoredClassName: container.querySelector('aside')?.className,
    restoredLabel: container
      .querySelector<HTMLButtonElement>(
        'button[aria-label="Show crit fullscreen"]',
      )
      ?.getAttribute('aria-pressed'),
  })

  expect(snapshot()).toEqual({
    fullscreenState: {
      className: 'crit-pane is-fullscreen',
      exitLabel: 'true',
    },
    restoredClassName: 'crit-pane',
    restoredLabel: 'false',
  })
})
