import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { PullRequestListPanel } from '#components/pull-request-list-panel'
import type { SessionPullRequest } from '#types'

const pullRequests: SessionPullRequest[] = [
  {
    number: 42,
    owner: 'sample-org',
    repo: 'sample-repo',
    title: 'Add a safe merge action',
    url: 'https://github.com/sample-org/sample-repo/pull/42',
    headSha: 'confirmed-head',
    state: 'Open',
    ciState: 'Successful',
    reviewStatus: 'done',
  },
]

let container: HTMLDivElement
let root: Root | null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  act(() => root?.unmount())
  container.remove()
  root = null
  vi.unstubAllGlobals()
})

function renderPanel(onRefresh = vi.fn()) {
  act(() => {
    root?.render(
      <PullRequestListPanel
        sessionId="sample-session"
        pullRequests={pullRequests}
        loading={false}
        error={null}
        onRefresh={onRefresh}
      />,
    )
  })
  return onRefresh
}

async function mergeSnapshot(result: unknown, status = 200) {
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(JSON.stringify(result), {
        status,
        headers: { 'Content-Type': 'application/json' },
      }),
    ),
  )
  vi.stubGlobal('fetch', fetcher)
  const onRefresh = renderPanel()

  await act(async () => {
    await userEvent.click(
      requireElement(container.querySelector('button'), 'Merge button'),
    )
  })
  const dialog = requireElement(
    container.querySelector('[role="alertdialog"]'),
    'merge confirmation',
  )
  const confirmedHead = dialog.querySelector('code')?.textContent ?? null
  await act(async () => {
    await userEvent.click(
      requireElement(
        dialog.querySelector('button:last-child'),
        'Confirm merge button',
      ),
    )
  })
  await waitForState(
    () =>
      container.querySelector('[role="status"]') !== null ||
      container.querySelector('[role="alert"]') !== null,
  )

  return {
    confirmedHead,
    requests: fetcher.mock.calls.map(([input, init]) => ({
      url: requestUrl(input),
      method: init?.method,
      body: init?.body,
    })),
    feedback:
      container.querySelector('[role="status"]')?.textContent ??
      container.querySelector('[role="alert"]')?.textContent,
    refreshCalls: onRefresh.mock.calls.length,
  }
}

function requireElement<T extends Element>(
  element: T | null,
  label: string,
): T {
  if (element === null) throw new Error(`Expected ${label} to be present`)
  return element
}

function requestUrl(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.href
  return input.url
}

async function waitForState(condition: () => boolean) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (condition()) return
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
  throw new Error('The expected panel state did not appear')
}

it('sends the confirmed head SHA and reports a successful merge', async () => {
  expect(await mergeSnapshot({ result: 'merged' })).toEqual({
    confirmedHead: 'confirmed-head',
    requests: [
      {
        url: '/api/sessions/sample-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'sample-org',
          repo: 'sample-repo',
          expectedHeadSha: 'confirmed-head',
        }),
      },
    ],
    feedback: 'PR #42 merged.',
    refreshCalls: 1,
  })
})

it('shows merge progress and blocks duplicate confirmation', async () => {
  let completePost: (response: Response) => void = () => {
    throw new Error('Expected the merge request to be pending')
  }
  const fetcher = vi.fn<typeof fetch>(
    () =>
      new Promise<Response>((resolve) => {
        completePost = resolve
      }),
  )
  vi.stubGlobal('fetch', fetcher)
  const onRefresh = renderPanel()
  await act(async () => {
    await userEvent.click(
      requireElement(container.querySelector('button'), 'Merge button'),
    )
  })
  const dialog = requireElement(
    container.querySelector('[role="alertdialog"]'),
    'merge confirmation',
  )
  await act(async () => {
    await userEvent.click(
      requireElement(
        dialog.querySelector('button:last-child'),
        'Confirm merge button',
      ),
    )
  })
  const beforeResponse = {
    confirmLabel: dialog.querySelector('button:last-child')?.textContent,
    confirmDisabled:
      dialog.querySelector<HTMLButtonElement>('button:last-child')?.disabled,
    cancelDisabled:
      dialog.querySelector<HTMLButtonElement>('button:first-child')?.disabled,
  }

  await act(async () => {
    completePost(Response.json({ result: 'merged' }))
    await new Promise((resolve) => window.setTimeout(resolve, 20))
  })
  const afterResponse = {
    feedback: container.querySelector('[role="status"]')?.textContent,
    refreshCalls: onRefresh.mock.calls.length,
  }

  const snapshot = () => ({ beforeResponse, afterResponse })
  expect(snapshot()).toEqual({
    beforeResponse: {
      confirmLabel: 'Merging…',
      confirmDisabled: true,
      cancelDisabled: true,
    },
    afterResponse: { feedback: 'PR #42 merged.', refreshCalls: 1 },
  })
})

it.each([
  [
    'already merged',
    { result: 'alreadyMerged' },
    'PR #42 was already merged.',
    1,
  ],
  [
    'queued',
    { result: 'enqueued', message: 'waiting for merge queue' },
    'PR #42 was added to the merge queue: waiting for merge queue',
    0,
  ],
])(
  'reports a %s response with the expected refresh behavior',
  async (_name, result, feedback, refreshCalls) => {
    expect(await mergeSnapshot(result)).toEqual({
      confirmedHead: 'confirmed-head',
      requests: [
        {
          url: '/api/sessions/sample-session/pull-requests/42/merge',
          method: 'POST',
          body: JSON.stringify({
            owner: 'sample-org',
            repo: 'sample-repo',
            expectedHeadSha: 'confirmed-head',
          }),
        },
      ],
      feedback,
      refreshCalls,
    })
  },
)

it('shows the server reason when the confirmed head changed', async () => {
  expect(
    await mergeSnapshot(
      {
        error: 'PR #42 head changed since confirmation.',
      },
      502,
    ),
  ).toEqual({
    confirmedHead: 'confirmed-head',
    requests: [
      {
        url: '/api/sessions/sample-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'sample-org',
          repo: 'sample-repo',
          expectedHeadSha: 'confirmed-head',
        }),
      },
    ],
    feedback: 'PR #42 head changed since confirmation.',
    refreshCalls: 0,
  })
})
