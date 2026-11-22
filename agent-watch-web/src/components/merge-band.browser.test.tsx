import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { MergeBand } from '#components/merge-band'
import type { PullRequest } from '#types'

type MergeBandRow = ComponentProps<typeof MergeBand>['rows'][number]

const pullRequest: PullRequest = {
  number: 42,
  state: 'Open',
  owner: 'example-org',
  repo: 'example-service',
  title: 'Show delivery status in the session header',
  headSha: 'fixture-head-sha',
  ciState: 'Successful',
  isCurrentBranch: true,
}

let container: HTMLDivElement
let root: Root | null = null

function render(rows: readonly MergeBandRow[]) {
  const onMerge = vi.fn()
  const onRetry = vi.fn()
  act(() => {
    root?.render(
      <MergeBand
        rows={rows}
        isMobile={false}
        onMerge={onMerge}
        onRetry={onRetry}
      />,
    )
  })
  return { onMerge, onRetry }
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('shows each merge state with a control only when it can be acted on', () => {
  render([
    { pullRequest, status: 'ready' },
    { pullRequest: { ...pullRequest, number: 43 }, status: 'merging' },
    {
      pullRequest: { ...pullRequest, number: 44 },
      status: 'mergeFailed',
      error: 'The base branch changed.',
    },
    {
      pullRequest: {
        ...pullRequest,
        number: 45,
        ciState: 'Failed',
      },
      status: 'mergeFailed',
      error: 'CI failed',
      canRetry: false,
    },
  ])

  const snapshot = () =>
    Array.from(
      container.querySelectorAll<HTMLElement>('.merge-band-row'),
      (row) => ({
        status: row.querySelector('.merge-band-heading strong')?.textContent,
        detail: row.querySelector('.merge-band-detail')?.textContent,
        action: (() => {
          const button = row.querySelector<HTMLButtonElement>('button')
          return button === null
            ? null
            : {
                label: button.textContent,
                disabled: button.disabled,
                ariaLabel: button.getAttribute('aria-label'),
              }
        })(),
      }),
    )

  expect(snapshot()).toEqual([
    {
      status: 'Ready to merge',
      detail: '#42 Show delivery status in the session header',
      action: {
        label: 'Merge',
        disabled: false,
        ariaLabel: 'Merge PR #42',
      },
    },
    {
      status: 'Merging…',
      detail: '#43 Show delivery status in the session header',
      action: {
        label: 'Merging…',
        disabled: true,
        ariaLabel: 'Merging… PR #43',
      },
    },
    {
      status: 'Merge failed',
      detail: '#44 The base branch changed.',
      action: {
        label: 'Retry',
        disabled: false,
        ariaLabel: 'Retry merging PR #44',
      },
    },
    {
      status: 'Merge failed',
      detail: '#45 CI failed',
      action: null,
    },
  ])
})

it('sends the selected pull request to the matching action', () => {
  const onMerge = vi.fn()
  const onRetry = vi.fn()
  act(() => {
    root?.render(
      <MergeBand
        rows={[
          { pullRequest, status: 'ready' },
          {
            pullRequest: { ...pullRequest, number: 43 },
            status: 'mergeFailed',
          },
        ]}
        isMobile={false}
        onMerge={onMerge}
        onRetry={onRetry}
      />,
    )
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Merge PR #42"]')
      ?.click()
    container
      .querySelector<HTMLButtonElement>('[aria-label="Retry merging PR #43"]')
      ?.click()
  })

  const callbackCalls = () => ({
    merge: onMerge.mock.calls,
    retry: onRetry.mock.calls,
  })
  expect(callbackCalls()).toEqual({
    merge: [[pullRequest]],
    retry: [[{ ...pullRequest, number: 43 }]],
  })
})
