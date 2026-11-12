import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { PullRequestListPanel } from '#components/pull-request-list-panel'
import type { SessionPullRequest } from '#types'

const pullRequests: SessionPullRequest[] = [
  {
    number: 42,
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
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(JSON.stringify({ result: 'merged' }), {
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
  expect(confirmedHead).toEqual('confirmed-head')
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
      container.querySelector('[role="status"]')?.textContent ===
      'PR #42 merged.',
  )

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([input, init]) => ({
      url: requestUrl(input),
      method: init?.method,
      body: init?.body,
    })),
    feedback: container.querySelector('[role="status"]')?.textContent,
    refreshCalls: onRefresh.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    requests: [
      {
        url: '/api/sessions/sample-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({ expectedHeadSha: 'confirmed-head' }),
      },
    ],
    feedback: 'PR #42 merged.',
    refreshCalls: 1,
  })
})

it('shows the server reason when the confirmed head changed', async () => {
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(
        JSON.stringify({ error: 'PR #42 head changed since confirmation.' }),
        {
          status: 502,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    ),
  )
  vi.stubGlobal('fetch', fetcher)
  const onRefresh = renderPanel()

  await act(async () => {
    await userEvent.click(
      requireElement(container.querySelector('button'), 'Merge button'),
    )
  })
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector('[role="alertdialog"] button:last-child'),
        'Confirm merge button',
      ),
    )
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'PR #42 head changed since confirmation.',
  )

  const snapshot = () => ({
    feedback: container.querySelector('[role="alert"]')?.textContent,
    refreshCalls: onRefresh.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    feedback: 'PR #42 head changed since confirmation.',
    refreshCalls: 0,
  })
})
