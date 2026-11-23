import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import type { WebUiLinkTarget } from '#components/markdown-session-links'
import type { ReviewDocumentSummary } from '#components/review-document-summary'
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
  reviewDocumentSummaries,
  isMobile = false,
}: {
  currentSession: Session
  statusPanelOpen?: boolean
  reviewDocumentSummaries?: readonly ReviewDocumentSummary[]
  isMobile?: boolean
}) {
  const summaries =
    reviewDocumentSummaries ??
    (currentSession.reviewDocumentId == null
      ? []
      : [
          {
            id: currentSession.reviewDocumentId,
            kind: 'draft' as const,
            target: {},
            isEditor: true,
            canSave: true,
            canFinish: true,
          },
        ])
  const panel = useConversationStatusPanel({
    session: currentSession,
    reviewDocumentSummaries: summaries,
    messages: [],
    backgroundTasks: [],
    statusPanelOpen,
    isMobile,
    onAction: () => Promise.resolve(true),
    onCloseStatusPanel: () => undefined,
  })
  return (
    <SessionStatusPanel
      tabs={panel.tabs}
      activeTabId={panel.activeTabId}
      onTabChange={panel.onTabChange}
      onClose={() => undefined}
      expanded={panel.isReviewExpanded}
      onCollapse={panel.onCollapseReview}
      size={isMobile ? 'phone' : 'desktop'}
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
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (condition()) return
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 10))
    })
  }
  if (condition()) return
  throw new Error('The expected review document state did not appear')
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.style.display = 'flex'
  container.style.width = '480px'
  container.style.height = '720px'
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

  expect(states).toBe('|\nReview1|Review1\n|')
})

it('shows linked open and completed tasks in the Tasks tab', () => {
  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{
          ...session,
          task: {
            id: 'example-open-task',
            number: 18,
            title: 'Improve request reliability',
            priority: 1,
          },
          tasks: [
            {
              id: 'example-open-task',
              number: 18,
              title: 'Improve request reliability',
              priority: 1,
            },
            {
              id: 'example-completed-task',
              number: 17,
              title: 'Document request behavior',
              priority: null,
              isClosed: true,
              updatedAt: new Date(Date.now() - 3_600_000).toISOString(),
            },
          ],
        }}
      />,
    )
  })

  const snapshot = () => ({
    tabs: Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
      (tab) => ({
        text: tab.textContent,
        selected: tab.getAttribute('aria-selected'),
      }),
    ),
    tasks: Array.from(
      container.querySelectorAll('.session-tasks-panel-item'),
      (task) => task.textContent,
    ),
    primaryControls: Array.from(
      container.querySelectorAll(
        '.session-tasks-panel-primary, .session-tasks-panel-primary-action',
      ),
      (control) => control.textContent,
    ),
  })

  expect(snapshot()).toEqual({
    tabs: [{ text: 'Tasks2', selected: 'true' }],
    tasks: [
      '●#18 Improve request reliabilityP1 · toolkit',
      '✓#17 Document request behavior 1h',
    ],
    primaryControls: ['Primary', 'Make primary'],
  })
})

it('shows linked tasks without adding a PRs tab', () => {
  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{
          ...session,
          task: {
            id: 'fixture-task',
            number: 17,
            title: 'Fixture task',
            priority: null,
          },
          pullRequest: { number: 42, state: 'Open' },
        }}
      />,
    )
  })

  expect(
    Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
      (tab) => tab.textContent,
    ),
  ).toEqual(['Tasks1'])
})

it.each(['Todo', 'Proposal'] as const)(
  'shows the legacy Crit tab for a needs_action %s session',
  (kind) => {
    act(() => {
      root?.render(
        <StatusPanelHarness
          currentSession={{
            ...session,
            kind,
            isWaitingForCritReview: true,
            critUrl: 'https://crit.example.test/review/fixture',
          }}
        />,
      )
    })

    const snapshot = () => ({
      tabs: Array.from(
        container.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
        (tab) => ({
          label: tab.textContent,
          selected: tab.getAttribute('aria-selected'),
        }),
      ),
      paneTitle: container.querySelector('.web-ui-pane-title')?.textContent,
    })

    expect(snapshot()).toEqual({
      tabs: [{ label: 'crit', selected: 'true' }],
      paneTitle: 'crit',
    })
  },
)

it('does not synthesize a legacy Crit tab outside needs_action', () => {
  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{
          ...session,
          category: 'in_progress',
          critUrl: 'https://crit.example.test/review/fixture',
        }}
      />,
    )
  })

  const snapshot = () => ({
    tabs: Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
      (tab) => tab.textContent,
    ),
    paneTitle:
      container.querySelector('.web-ui-pane-title')?.textContent ?? null,
  })

  expect(snapshot()).toEqual({ tabs: [], paneTitle: null })
})

it('fetches and renders the review document when the status panel opens', async () => {
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(
        JSON.stringify({
          id: 'fixture-review-a',
          revision: 'fixture-revision-a',
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
    title: container.querySelector<HTMLInputElement>(
      '.review-document-frontmatter input[type="text"]',
    )?.value,
    markdown: container.querySelector('.review-document-body strong')
      ?.textContent,
  })
  const panel = () => ({ closedPanelRequests, ...openPanel() })
  expect(panel()).toEqual({
    closedPanelRequests: [],
    requests: [
      '/api/sessions/fixture-session/review-documents/fixture-review-a',
    ],
    selectedTab: 'Review1',
    title: 'Fixture pull request',
    markdown: 'Fixture draft body',
  })
})

it('loads each selected draft and restores the previous draft when switching back', async () => {
  const requests: string[] = []
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    requests.push(url)
    const isSecondDraft = url.endsWith('/fixture-review-b')
    return Promise.resolve(
      new Response(
        JSON.stringify({
          id: isSecondDraft ? 'fixture-review-b' : 'fixture-review-a',
          revision: 'fixture-revision',
          frontmatter: {
            title: isSecondDraft ? 'Second draft' : 'First draft',
          },
          body: isSecondDraft ? 'Second draft body' : 'First draft body',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  const reviewDocumentSummaries: ReviewDocumentSummary[] = [
    {
      id: 'fixture-review-a',
      kind: 'pr_draft',
      target: { repository: 'example/service', branch: 'first' },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    {
      id: 'fixture-review-b',
      kind: 'issue_comment',
      target: { issueNumber: 28, title: 'Improve retries' },
      isEditor: false,
      canSave: false,
      canFinish: false,
    },
  ]

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{
          ...session,
          reviewDocumentId: 'fixture-review-a',
          reviewDocumentCount: 2,
        }}
        reviewDocumentSummaries={reviewDocumentSummaries}
        statusPanelOpen
      />,
    )
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-body')?.textContent ===
      'First draft body',
  )
  await act(async () => {
    await page
      .getByRole('button', { name: 'Issue comment #28 · Improve retries' })
      .click()
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-body')?.textContent ===
      'Second draft body',
  )
  await act(async () => {
    await page
      .getByRole('button', { name: 'PR draft example/service · first' })
      .click()
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-body')?.textContent ===
      'First draft body',
  )

  const selectedDraftSnapshot = () => ({
    requests,
    selectedDraft: container.querySelector('.review-document-tab--selected')
      ?.textContent,
    body: container.querySelector('.review-document-body')?.textContent,
    reviewTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
  })

  expect(selectedDraftSnapshot()).toEqual({
    requests: [
      '/api/sessions/fixture-session/review-documents/fixture-review-a',
      '/api/sessions/fixture-session/review-documents/fixture-review-b',
    ],
    selectedDraft: 'PR draftexample/service · first',
    body: 'First draft body',
    reviewTab: 'Review2',
  })
})

it('expands the Review panel on desktop and hides the control on phone', async () => {
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(
        JSON.stringify({
          id: 'fixture-review-a',
          revision: 'fixture-revision',
          frontmatter: {},
          body: 'Readable draft body',
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
        statusPanelOpen
      />,
    )
  })
  await waitForState(
    () => container.querySelector('.review-document-body') !== null,
  )
  await act(async () => {
    await page.getByRole('button', { name: 'Open draft full width' }).click()
  })
  const expandedPanel = container
    .querySelector('.session-status-panel')
    ?.getAttribute('data-expanded')
  await act(async () => {
    await page.getByRole('button', { name: 'Exit full width review' }).click()
  })
  const collapsedPanel = container
    .querySelector('.session-status-panel')
    ?.getAttribute('data-expanded')

  act(() => {
    root?.render(
      <StatusPanelHarness
        currentSession={{ ...session, reviewDocumentId: 'fixture-review-a' }}
        isMobile
        statusPanelOpen
      />,
    )
  })

  const expansionSnapshot = () => ({
    expandedPanel,
    collapsedPanel,
    phoneExpandControls: container.querySelectorAll('.review-document-expand')
      .length,
  })

  expect(expansionSnapshot()).toEqual({
    expandedPanel: 'true',
    collapsedPanel: 'false',
    phoneExpandControls: 0,
  })
})

it('preserves unsaved review edits when a preview link opens a web UI', async () => {
  const webUi: WebUiLinkTarget = {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/',
  }
  const fetcher = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      new Response(
        JSON.stringify({
          id: 'fixture-review-a',
          revision: 'fixture-revision-a',
          frontmatter: { title: 'Saved title' },
          body: '[Open canvas page](http://localhost:43123/studio/pages/sample#page-fixture)',
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  function ReviewLinkHarness() {
    const panel = useConversationStatusPanel({
      session: {
        ...session,
        reviewDocumentId: 'fixture-review-a',
        webUis: [webUi],
      },
      reviewDocumentSummaries: [
        {
          id: 'fixture-review-a',
          kind: 'draft',
          target: {},
          isEditor: true,
          canSave: true,
          canFinish: true,
        },
      ],
      messages: [],
      backgroundTasks: [],
      statusPanelOpen: true,
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <MarkdownSessionLinksProvider
        sessions={[]}
        webUis={[webUi]}
        onOpenWebUi={panel.onOpenWebUiInFullscreen}
      >
        <SessionStatusPanel
          tabs={panel.tabs}
          activeTabId={panel.activeTabId}
          onTabChange={panel.onTabChange}
          onClose={() => undefined}
        />
      </MarkdownSessionLinksProvider>
    )
  }

  act(() => root?.render(<ReviewLinkHarness />))
  await act(async () => {
    await page.getByRole('tab', { name: 'Review' }).click()
  })
  await waitForState(
    () => container.querySelector('.review-document-preview-column a') !== null,
  )
  await act(async () => {
    await page.getByRole('textbox', { name: 'title' }).fill('Unsaved title')
    await page.getByRole('button', { name: 'Preview' }).click()
    await page.getByRole('link', { name: 'Open canvas page' }).click()
  })
  await waitForState(
    () =>
      container.querySelector('iframe')?.getAttribute('src') ===
      'http://localhost:43123/studio/pages/sample#page-fixture',
  )
  await act(async () => {
    await page
      .getByRole('button', { name: 'Exit Canvas editor fullscreen' })
      .click()
    await page.getByRole('tab', { name: 'Review' }).click()
  })
  await waitForState(
    () =>
      container.querySelector<HTMLInputElement>(
        '.review-document-frontmatter input[type="text"]',
      ) !== null,
  )
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  function getState() {
    return {
      requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
      title: container.querySelector<HTMLInputElement>(
        '.review-document-frontmatter input[type="text"]',
      )?.value,
    }
  }
  expect(getState()).toEqual({
    requests: [
      '/api/sessions/fixture-session/review-documents/fixture-review-a',
    ],
    title: 'Unsaved title',
  })
})

it('saves the edited document and sends the same edits before finishing review', async () => {
  const requests: { url: string; method: string; body: unknown }[] = []
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    const method = init?.method ?? 'GET'
    const body =
      typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : null
    requests.push({ url, method, body })
    return Promise.resolve(
      new Response(
        JSON.stringify(
          method === 'GET'
            ? {
                id: 'fixture-review-a',
                revision: 'fixture-revision-a',
                frontmatter: { title: 'Fixture title', enabled: false },
                body: 'Fixture body.',
              }
            : { success: true, revision: 'fixture-revision-b' },
        ),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      ),
    )
  })
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
    () => container.querySelector('.review-document-body') !== null,
  )

  await act(async () => {
    await page.getByRole('textbox', { name: 'title' }).fill('Edited title')
  })
  await act(async () => {
    await page.getByRole('checkbox', { name: 'enabled' }).click()
  })
  await act(async () => {
    await page.getByRole('button', { name: 'Edit' }).click()
  })
  await act(async () => {
    await page
      .getByRole('textbox', { name: 'Markdown body' })
      .fill('Edited body.')
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-editor .cm-content')
        ?.textContent === 'Edited body.',
  )
  await act(async () => {
    await page.getByRole('button', { name: 'Preview' }).click()
  })
  await waitForState(
    () =>
      container.querySelector('.review-document-body')?.textContent ===
      'Edited body.',
  )
  expect(container.querySelector('.review-document-status')?.textContent).toBe(
    'Unsaved changes',
  )
  await act(async () => {
    await page.getByRole('button', { name: 'Save' }).click()
  })
  await waitForState(
    () =>
      requests.length === 2 &&
      container.querySelector('.review-document-status')?.textContent ===
        'Saved',
  )
  await act(async () => {
    await page.getByRole('button', { name: 'Finish review' }).click()
  })
  await waitForState(() => requests.length === 3)

  expect(requests).toEqual([
    {
      url: '/api/sessions/fixture-session/review-documents/fixture-review-a',
      method: 'GET',
      body: null,
    },
    {
      url: '/api/sessions/fixture-session/review-document/save',
      method: 'POST',
      body: {
        id: 'fixture-review-a',
        revision: 'fixture-revision-a',
        frontmatter: { title: 'Edited title', enabled: true },
        body: 'Edited body.',
      },
    },
    {
      url: '/api/sessions/fixture-session/review-document/finish',
      method: 'POST',
      body: {
        id: 'fixture-review-a',
        revision: 'fixture-revision-b',
        frontmatter: { title: 'Edited title', enabled: true },
        body: 'Edited body.',
      },
    },
  ])
})

it.each([
  { action: 'save', button: 'Save' },
  { action: 'finish', button: 'Finish review' },
])(
  'shows the reload instruction and keeps edits after a $action conflict',
  async ({ action, button }) => {
    const requests: { url: string; method: string; body: unknown }[] = []
    const fetcher = vi.fn<typeof fetch>((input, init) => {
      const url = requestUrl(input)
      const method = init?.method ?? 'GET'
      const body =
        typeof init?.body === 'string'
          ? (JSON.parse(init.body) as unknown)
          : null
      requests.push({ url, method, body })
      if (method === 'GET') {
        return Promise.resolve(
          new Response(
            JSON.stringify({
              id: 'fixture-review-a',
              revision: 'fixture-revision-a',
              frontmatter: { title: 'Fixture title' },
              body: 'Fixture body.',
            }),
            { status: 200, headers: { 'Content-Type': 'application/json' } },
          ),
        )
      }
      return Promise.resolve(new Response(null, { status: 409 }))
    })
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
      () => container.querySelector('.review-document-body') !== null,
    )
    await act(async () => {
      await page.getByRole('textbox', { name: 'title' }).fill('Edited title')
      await page.getByRole('button', { name: 'Edit' }).click()
      await page
        .getByRole('textbox', { name: 'Markdown body' })
        .fill('Edited body.')
    })
    await waitForState(
      () =>
        container.querySelector('.review-document-body')?.textContent ===
        'Edited body.',
    )
    await act(async () => {
      await page.getByRole('button', { name: 'Preview' }).click()
    })
    await act(async () => {
      await page.getByRole('button', { name: button }).click()
    })
    await waitForState(() => container.querySelector('[role="alert"]') !== null)

    const snapshot = () => ({
      alert: container.querySelector('[role="alert"]')?.textContent,
      title: container.querySelector<HTMLInputElement>(
        '.review-document-frontmatter input',
      )?.value,
      body: container.querySelector('.review-document-editor .cm-content')
        ?.textContent,
      status: container.querySelector('.review-document-status')?.textContent,
      requests,
    })

    expect(snapshot()).toEqual({
      alert:
        'This draft changed since you opened it. Your edits are still here; copy them before reloading to see the latest version.',
      title: 'Edited title',
      body: 'Edited body.',
      status:
        'This draft changed since you opened it. Your edits are still here; copy them before reloading to see the latest version.',
      requests: [
        {
          url: '/api/sessions/fixture-session/review-documents/fixture-review-a',
          method: 'GET',
          body: null,
        },
        {
          url: `/api/sessions/fixture-session/review-document/${action}`,
          method: 'POST',
          body: {
            id: 'fixture-review-a',
            revision: 'fixture-revision-a',
            frontmatter: { title: 'Edited title' },
            body: 'Edited body.',
          },
        },
      ],
    })
  },
)

it('refetches the review document when its active ID changes', async () => {
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          id: 'fixture-review-a',
          revision: 'fixture-revision-a',
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
          revision: 'fixture-revision-b',
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
    title: container.querySelector<HTMLInputElement>(
      '.review-document-frontmatter input[type="text"]',
    )?.value,
    body: container.querySelector('.review-document-body')?.textContent,
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/fixture-session/review-documents/fixture-review-a',
      '/api/sessions/fixture-session/review-documents/fixture-review-b',
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
    requests: [
      '/api/sessions/fixture-session/review-documents/fixture-review-a',
    ],
    alert: 'Could not load the draft: Fixture request failed',
  })
})

it('opens a linked web UI URL in its fullscreen status tab', async () => {
  const webUi: WebUiLinkTarget = {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/',
  }
  const linkUrl = 'http://localhost:43123/studio/pages/sample#page-fakehash'
  function LinkHarness() {
    const panel = useConversationStatusPanel({
      session: { ...session, webUis: [webUi] },
      messages: [],
      backgroundTasks: [],
      statusPanelOpen: true,
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <>
        <button
          type="button"
          onClick={() => {
            panel.onOpenWebUiInFullscreen(webUi, linkUrl)
          }}
        >
          Open linked page
        </button>
        <SessionStatusPanel
          tabs={panel.tabs}
          activeTabId={panel.activeTabId}
          onTabChange={panel.onTabChange}
          onClose={() => undefined}
        />
      </>
    )
  }

  act(() => root?.render(<LinkHarness />))
  await act(async () => {
    await page.getByRole('button', { name: 'Open linked page' }).click()
  })

  const snapshot = () => ({
    activeTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    fullscreen: container
      .querySelector('.web-ui-pane')
      ?.classList.contains('is-fullscreen'),
    iframe: container.querySelector('iframe')?.getAttribute('src') ?? null,
  })
  expect(snapshot()).toEqual({
    activeTab: 'Canvas editor',
    fullscreen: true,
    iframe: 'http://localhost:43123/studio/pages/sample#page-fakehash',
  })
})

it('opens crit through the shared fullscreen web UI request', async () => {
  const crit: WebUiLinkTarget = {
    name: 'crit',
    url: 'http://localhost:43123/review/sample?token=local#local-page',
    exposedUrl:
      'https://crit.example.invalid/review/sample?token=exposed#exposed-page',
  }
  function CritHarness() {
    const panel = useConversationStatusPanel({
      session: { ...session, webUis: [crit] },
      messages: [],
      backgroundTasks: [],
      statusPanelOpen: true,
      hostname: 'watch.example.invalid',
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <>
        <button type="button" onClick={panel.onOpenCritInFullscreen}>
          Review in crit
        </button>
        <SessionStatusPanel
          tabs={panel.tabs}
          activeTabId={panel.activeTabId}
          onTabChange={panel.onTabChange}
          onClose={() => undefined}
        />
      </>
    )
  }

  act(() => root?.render(<CritHarness />))
  await act(async () => {
    await page.getByRole('button', { name: 'Review in crit' }).click()
  })

  const snapshot = () => ({
    activeTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    fullscreen: container
      .querySelector('.web-ui-pane')
      ?.classList.contains('is-fullscreen'),
    iframe: container.querySelector('iframe')?.getAttribute('src') ?? null,
  })
  expect(snapshot()).toEqual({
    activeTab: 'crit',
    fullscreen: true,
    iframe:
      'https://crit.example.invalid/review/sample?token=exposed#exposed-page',
  })
})

it('opens a linked web UI as a tabless mobile page and returns to conversation', async () => {
  const webUi: WebUiLinkTarget = {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/',
  }
  const linkUrl = 'http://localhost:43123/studio/pages/sample#page-fakehash'
  function MobileLinkHarness() {
    const panel = useConversationStatusPanel({
      session: { ...session, webUis: [webUi] },
      messages: [],
      backgroundTasks: [],
      isMobile: true,
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <>
        <button
          type="button"
          onClick={() => {
            panel.onOpenWebUiInFullscreen(webUi, linkUrl)
          }}
        >
          Open linked page
        </button>
        {panel.mobileWebUi}
      </>
    )
  }

  act(() => root?.render(<MobileLinkHarness />))
  await act(async () => {
    await page.getByRole('button', { name: 'Open linked page' }).click()
  })
  const opened = {
    dialog: container
      .querySelector('[role="dialog"]')
      ?.getAttribute('aria-label'),
    tabCount: container.querySelectorAll('[role="tab"]').length,
    iframe: container.querySelector('iframe')?.getAttribute('src') ?? null,
  }
  await act(async () => {
    await page.getByRole('button', { name: 'Close web UI' }).click()
  })

  const snapshot = () => ({
    opened,
    returned: container.querySelector('[role="dialog"]') === null,
  })
  expect(snapshot()).toEqual({
    opened: {
      dialog: 'Canvas editor',
      tabCount: 0,
      iframe: 'http://localhost:43123/studio/pages/sample#page-fakehash',
    },
    returned: true,
  })
})
