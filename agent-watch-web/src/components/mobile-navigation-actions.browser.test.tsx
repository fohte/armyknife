import '#index.css'

import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { AsyncActionFeedbackProvider } from '#async-action-feedback'
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

  const snapshot = () => ({
    newSessionCalls: onNewSession.mock.calls,
    activityButtonVisible:
      container.querySelector('.async-action-activity-button') !== null,
    activityButtonLabel: container
      .querySelector('.async-action-activity-button')
      ?.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    newSessionCalls: [[]],
    activityButtonVisible: true,
    activityButtonLabel: 'Activity, no actions yet',
  })
})

it('opens empty Activity from the mobile header', async () => {
  root = createRoot(container)
  await act(async () => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <MobileNavigationActions
          onOpenNavigation={() => undefined}
          onOpenPalette={() => undefined}
        />
      </AsyncActionFeedbackProvider>,
    )
    return Promise.resolve()
  })

  const activityButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-button',
  )
  if (activityButton === null) {
    throw new Error('The mobile header Activity button is missing')
  }

  await act(async () => {
    await userEvent.click(activityButton)
  })

  const activity = container.querySelector(
    '[role="dialog"][aria-label="Activity"]',
  )
  const snapshot = () => ({
    activityButtonLabel: activityButton.getAttribute('aria-label'),
    activityText: activity?.textContent ?? null,
    clearFinishedVisible:
      activity !== null && activity.querySelector('header button') !== null,
    emptyMessages: Array.from(
      activity?.querySelectorAll('.async-action-empty p') ?? [],
      (paragraph) => paragraph.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    activityButtonLabel: 'Activity, no actions yet',
    activityText:
      'ActivityNo recent activity.Merge and Close results appear here.Close',
    clearFinishedVisible: false,
    emptyMessages: [
      'No recent activity.',
      'Merge and Close results appear here.',
    ],
  })
})
