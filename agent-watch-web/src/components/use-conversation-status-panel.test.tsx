import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import type { Session } from '#types'

const session: Session = {
  id: 'fixture-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Review',
  title: 'Fixture session',
  preview: null,
  elapsedSeconds: 30,
  unread: false,
  task: null,
  repo: 'fixture/toolkit',
  project: 'Fixture toolkit',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  reviewDocumentId: null,
}

let container: HTMLDivElement
let root: Root | null = null

function StatusPanelHarness({
  currentSession,
  statusPanelOpen = false,
}: {
  currentSession: Session
  statusPanelOpen?: boolean
}) {
  const panel = useConversationStatusPanel({
    session: currentSession,
    messages: [],
    backgroundTasks: [],
    statusPanelOpen,
    onAction: () => Promise.resolve(true),
    onCloseStatusPanel: () => undefined,
  })
  return (
    <SessionStatusPanel
      tabs={panel.tabs}
      activeTabId={panel.activeTabId}
      onTabChange={panel.onTabChange}
      onClose={() => undefined}
    />
  )
}

function requestUrl(input: RequestInfo | URL): string {
  return typeof input === 'string'
    ? input
    : input instanceof URL
      ? input.href
      : input.url
}

async function waitForState(condition: () => boolean) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (condition()) return
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    })
  }
  throw new Error('The expected review document state did not appear')
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
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('shows the Review tab only for sessions with an active review document', () => {
  const render = (currentSession: Session) => {
    act(() => {
      root?.render(<StatusPanelHarness currentSession={currentSession} />)
    })
    return {
      tabs: Array.from(
        container.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
        (tab) => tab.textContent,
      ),
      selectedTab: container.querySelector('[aria-selected="true"]')
        ?.textContent,
    }
  }

  const states = [
    render(session),
    render({ ...session, reviewDocumentId: 'fixture-review-a' }),
    render({ ...session, reviewDocumentId: null }),
  ]
    .map(({ tabs, selectedTab }) => `${tabs.join(',')}|${selectedTab ?? ''}`)
    .join('\n')

  expect(states).toBe('|\nReview|Review\n|')
})

it('fetches and renders the review document when the status panel opens', async () => {
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(
        JSON.stringify({
          id: 'fixture-review-a',
          frontmatter: { title: 'Fixture pull request' },
          body: '**Fixture draft body**',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{ ...session, reviewDocumentId: 'fixture-review-a' }}
      />,
    )
  })
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  const closedPanelRequests = fetcher.mock.calls.map(([input]) =>
    requestUrl(input),
  )

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{ ...session, reviewDocumentId: 'fixture-review-a' }}
        statusPanelOpen
      />,
    )
  })
  await waitForState(
    () => container.querySelector('.review-document-body') !== null,
  )

  const openPanel = () => ({
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    selectedTab: container.querySelector('[aria-selected="true"]')?.textContent,
    title: container.querySelector('.review-document-field-value')?.textContent,
    markdown: container.querySelector('.review-document-body strong')
      ?.textContent,
  })
  const panel = () => ({ closedPanelRequests, ...openPanel() })
  expect(panel()).toEqual({
    closedPanelRequests: [],
    requests: ['/api/sessions/fixture-session/review-document'],
    selectedTab: 'Review',
    title: 'Fixture pull request',
    markdown: 'Fixture draft body',
  })
})

it('refetches the review document when its active ID changes', async () => {
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          id: 'fixture-review-a',
          frontmatter: { title: 'First fixture' },
          body: 'First body',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    )
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          id: 'fixture-review-b',
          frontmatter: { title: 'Second fixture' },
          body: 'Second body',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    )
  vi.stubGlobal('fetch', fetcher)

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{ ...session, reviewDocumentId: 'fixture-review-a' }}
        statusPanelOpen
      />,
    )
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-body')?.textContent ===
      'First body',
  )

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{ ...session, reviewDocumentId: 'fixture-review-b' }}
        statusPanelOpen
      />,
    )
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-body')?.textContent ===
      'Second body',
  )

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    title: container.querySelector('.review-document-field-value')?.textContent,
    body: container.querySelector('.review-document-body')?.textContent,
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/fixture-session/review-document',
      '/api/sessions/fixture-session/review-document',
    ],
    title: 'Second fixture',
    body: 'Second body',
  })
})

it('shows an alert when the review document request fails', async () => {
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(JSON.stringify({ error: 'Fixture request failed' }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      }),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{ ...session, reviewDocumentId: 'fixture-review-a' }}
        statusPanelOpen
      />,
    )
  })
  await waitForState(() => container.querySelector('[role="alert"]') !== null)

  const result = () => ({
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    alert: container.querySelector('[role="alert"]')?.textContent,
  })
  expect(result()).toEqual({
    requests: ['/api/sessions/fixture-session/review-document'],
    alert: 'Could not load the review document: Fixture request failed',
  })
})
