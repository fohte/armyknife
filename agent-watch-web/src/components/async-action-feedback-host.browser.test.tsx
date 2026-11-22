import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import type { AsyncAction } from '#async-action-feedback'
import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'

const failedMerge: AsyncAction = {
  id: 1,
  sessionId: 'example-session',
  sessionTitle: 'Example session',
  kind: 'merge',
  status: 'mergeFailed',
  pullRequestNumber: 42,
  pullRequestOwner: 'example-org',
  pullRequestRepo: 'example-repo',
  error: 'The example branch changed after confirmation.',
  createdAt: 1_800_000_000_003,
}

const phoneActivityActions: AsyncAction[] = [
  failedMerge,
  {
    id: 2,
    sessionId: 'example-session',
    sessionTitle: 'Example session',
    kind: 'merge',
    pullRequestNumber: 43,
    status: 'merged',
    createdAt: 1_800_000_000_002,
  },
  {
    id: 3,
    sessionId: 'example-session',
    sessionTitle: 'Example session',
    kind: 'merge',
    pullRequestNumber: 44,
    status: 'merging',
    createdAt: 1_800_000_000_001,
  },
] satisfies readonly AsyncAction[]

function ActivityHarness({
  actions,
  calls,
}: {
  actions: readonly AsyncAction[]
  calls: string[]
}) {
  const [activityOpen, setActivityOpen] = useState(true)
  return (
    <AsyncActionFeedbackHost
      actions={actions}
      toastIds={[]}
      activityOpen={activityOpen}
      toastBottomOffset={null}
      selectedActionId={actions[0]?.id ?? null}
      retry={() => {
        calls.push('retry')
      }}
      dismissToast={() => undefined}
      clearFinished={() => undefined}
      openActivity={() => undefined}
      closeActivity={() => {
        calls.push('close')
        setActivityOpen(false)
      }}
      closeWithWorktree={() => undefined}
      openInTerminal={() => undefined}
    />
  )
}

let root: Root | null = null
let container: HTMLDivElement

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  root = null
  document.body.innerHTML = ''
})

it('closes Activity before retrying a failed action', () => {
  const calls: string[] = []
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(<ActivityHarness actions={[failedMerge]} calls={calls} />)
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.async-action-activity-list button')
      ?.click()
  })

  const snapshot = () => ({
    calls,
    activityOpen: container.querySelector('[aria-label="Activity"]') !== null,
  })
  expect(snapshot()).toEqual({
    calls: ['close', 'retry'],
    activityOpen: false,
  })
})

it('fits the empty phone sheet to its content and shows the dimmed backdrop', async () => {
  await page.viewport(390, 844)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(<ActivityHarness actions={[]} calls={[]} />)
  })

  const dialog = container.querySelector<HTMLElement>('.async-action-activity')
  const backdrop = container.querySelector<HTMLElement>(
    '.async-action-activity-backdrop',
  )
  const close = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-close',
  )
  if (dialog === null || backdrop === null || close === null) {
    throw new Error('The empty phone Activity sheet is missing')
  }

  const snapshot = () => ({
    contentSized: dialog.getBoundingClientRect().height < 300,
    closeButtonHeight: close.getBoundingClientRect().height,
    backdropColor: getComputedStyle(backdrop).backgroundColor,
  })
  expect(snapshot()).toEqual({
    contentSized: true,
    closeButtonHeight: 44,
    backdropColor: 'rgba(0, 0, 0, 0.55)',
  })
})

it('shows the full failed action and Retry in the expanded phone sheet', async () => {
  await page.viewport(390, 844)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(<ActivityHarness actions={phoneActivityActions} calls={[]} />)
  })

  const dialog = container.querySelector<HTMLElement>('.async-action-activity')
  const list = container.querySelector<HTMLElement>(
    '.async-action-activity-list',
  )
  const error = container.querySelector<HTMLElement>('.async-action-error')
  const retry = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-list button',
  )
  if (dialog === null || list === null || error === null || retry === null) {
    throw new Error('The failed phone Activity entry is missing')
  }

  const dialogRect = dialog.getBoundingClientRect()
  const retryRect = retry.getBoundingClientRect()
  const content = container.querySelector<HTMLElement>(
    '.async-action-activity-list .async-action-item-content',
  )
  if (content === null) {
    throw new Error('The failed phone Activity content is missing')
  }
  const contentRect = content.getBoundingClientRect()
  const snapshot = () => ({
    fullError: error.textContent,
    retryLeftAligned: retryRect.left === contentRect.left,
    retryUsesCompactWidth: retryRect.width < contentRect.width / 3,
    retryVisible:
      retryRect.height >= 44 &&
      retryRect.top >= dialogRect.top &&
      retryRect.bottom <= dialogRect.bottom,
    allRowsFit: list.scrollHeight === list.clientHeight,
    sheetUnderViewport: dialogRect.height < 844,
  })
  expect(snapshot()).toEqual({
    fullError: 'The example branch changed after confirmation.',
    retryLeftAligned: true,
    retryUsesCompactWidth: true,
    retryVisible: true,
    allRowsFit: true,
    sheetUnderViewport: true,
  })
})

it('positions the toast stack at the supplied screen offset', () => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackHost
        actions={[failedMerge]}
        toastIds={[failedMerge.id]}
        activityOpen={false}
        toastBottomOffset={200}
        selectedActionId={null}
        retry={() => undefined}
        dismissToast={() => undefined}
        clearFinished={() => undefined}
        openActivity={() => undefined}
        closeActivity={() => undefined}
        closeWithWorktree={() => undefined}
        openInTerminal={() => undefined}
      />,
    )
  })

  const stack = document.querySelector<HTMLElement>('.async-action-toast-stack')
  expect(stack === null ? null : getComputedStyle(stack).bottom).toEqual(
    '200px',
  )
})
