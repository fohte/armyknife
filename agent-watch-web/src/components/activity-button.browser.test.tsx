import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

import { AsyncActionFeedbackProvider } from '#async-action-feedback'
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
    label: 'an idle phone drawer entry',
    props: { activityCount: 0, drawer: true },
    expected: {
      ariaExpanded: 'false',
      ariaHasPopup: 'dialog',
      ariaLabel: 'Activity, no actions yet',
      count: null,
      className: 'async-action-activity-button is-drawer is-empty',
      iconSize: '16',
      height: '44px',
      borderTopWidth: '0px',
    },
  },
  {
    label: 'a running phone drawer entry',
    props: { activityCount: 2, drawer: true, isRunning: true },
    expected: {
      ariaExpanded: 'false',
      ariaHasPopup: 'dialog',
      ariaLabel: 'Activity, 1 running',
      count: '2',
      className: 'async-action-activity-button is-drawer',
      iconSize: '16',
      height: '44px',
      borderTopWidth: '0px',
    },
  },
  {
    label: 'a failed phone drawer entry',
    props: { activityCount: 2, drawer: true, failed: true },
    expected: {
      ariaExpanded: 'false',
      ariaHasPopup: 'dialog',
      ariaLabel: 'Activity, failed actions',
      count: '2',
      className: 'async-action-activity-button is-drawer',
      iconSize: '16',
      height: '44px',
      borderTopWidth: '0px',
    },
  },
])('$label shows the matching state', ({ props, expected }) => {
  root = createRoot(container)
  act(() => {
    root?.render(<ActivityButton {...props} onOpenActivity={() => undefined} />)
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
          iconSize: button.querySelector('svg')?.getAttribute('width'),
          height: getComputedStyle(button).height,
          borderTopWidth: getComputedStyle(button).borderTopWidth,
          count:
            button.querySelector('.async-action-activity-state')?.textContent ??
            null,
          className: button.className,
        }

  expect(snapshot()).toEqual(expected)
})

it('closes empty Activity with Escape on desktop', async () => {
  await page.viewport(1280, 800)
  root = createRoot(container)
  await act(async () => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <ActivityButton />
      </AsyncActionFeedbackProvider>,
    )
    return Promise.resolve()
  })

  const activityButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-button',
  )
  if (activityButton === null) {
    throw new Error('The Activity button is missing')
  }

  await act(async () => {
    await userEvent.click(activityButton)
  })

  const closeButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-close',
  )
  if (closeButton === null) {
    throw new Error('The Activity close button is missing')
  }
  const closeButtonDisplay = getComputedStyle(closeButton).display

  await act(async () => {
    await userEvent.keyboard('{Escape}')
  })

  const snapshot = () => ({
    closeButtonDisplay,
    activityOpen: container.querySelector('[role="dialog"]') !== null,
    buttonExpanded: activityButton.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    closeButtonDisplay: 'none',
    activityOpen: false,
    buttonExpanded: 'false',
  })
})
