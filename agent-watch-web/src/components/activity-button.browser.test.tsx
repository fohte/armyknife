import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ActivityButton } from '#components/activity-button'

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

it.each([
  {
    label: 'an empty Activity button',
    activityCount: 0,
    expected: {
      ariaExpanded: 'false',
      ariaHasPopup: 'dialog',
      ariaLabel: 'Activity, no actions yet',
      count: null,
      className: 'async-action-activity-button is-empty',
    },
  },
  {
    label: 'Activity with one result',
    activityCount: 1,
    expected: {
      ariaExpanded: 'false',
      ariaHasPopup: 'dialog',
      ariaLabel: 'Activity, results available',
      count: '1',
      className: 'async-action-activity-button',
    },
  },
])(
  '$label remains available in the navigation',
  ({ activityCount, expected }) => {
    root = createRoot(container)
    act(() => {
      root?.render(
        <ActivityButton
          activityCount={activityCount}
          onOpenActivity={() => undefined}
        />,
      )
    })

    const button = container.querySelector<HTMLButtonElement>(
      '.async-action-activity-button',
    )
    const snapshot = () =>
      button === null
        ? null
        : {
            ariaExpanded: button.getAttribute('aria-expanded'),
            ariaHasPopup: button.getAttribute('aria-haspopup'),
            ariaLabel: button.getAttribute('aria-label'),
            count:
              button.querySelector('.async-action-activity-state')
                ?.textContent ?? null,
            className: button.className,
          }

    expect(snapshot()).toEqual(expected)
  },
)
