import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  AsyncActionFeedbackProvider,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'
import { ConversationPaneHeader } from '#components/conversation-pane-header'
import { SessionOperationProvider } from '#session-operation-context'
import type { Session, SessionActions } from '#types'
import { usePendingMergeRetryNavigation } from '#use-pending-merge-retry-navigation'

const { copySessionIdMock } = vi.hoisted(() => ({
  copySessionIdMock: vi.fn(),
}))

vi.mock('#components/copy-session-id-control', () => ({
  CopySessionIdControl: () => null,
}))

vi.mock('#components/use-copy-session-id', () => ({
  useCopySessionId: () => ({
    status: 'idle',
    copySessionId: copySessionIdMock,
  }),
}))

const session: Session = {
  id: 'original-fixture-session',
  engine: 'claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Message',
  title: 'Original fixture session',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'example/project',
  project: 'project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const otherSession: Session = {
  ...session,
  id: 'other-fixture-session',
  title: 'Other fixture session',
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: false, reject: false },
  interrupt: false,
  openInTerminal: false,
  rewind: false,
}

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function RetryNavigationHarness({
  onAction,
}: {
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
}) {
  const feedback = useOptionalAsyncActionFeedback()
  const [route, setRoute] = useState({
    view: 'inbox' as 'inbox' | 'board',
    selectedId: session.id,
    conversationOpen: true,
  })
  const [latestHead, setLatestHead] = useState(false)
  usePendingMergeRetryNavigation({
    isMobile: true,
    view: route.view,
    selectedId: route.selectedId,
    conversationOpen: route.conversationOpen,
    newSessionMode: null,
    sessions: [session, otherSession],
    openSessionInInbox: (id) => {
      setRoute({ view: 'inbox', selectedId: id, conversationOpen: true })
    },
  })

  const currentSession =
    route.selectedId === session.id
      ? {
          ...session,
          pullRequest: {
            number: 42,
            state: 'Open' as const,
            owner: 'example-org',
            repo: 'example-repo',
            url: 'https://github.com/example-org/example-repo/pull/42',
            headSha: latestHead ? 'current-fixture-head' : 'old-fixture-head',
            ciState: 'Successful' as const,
            isCurrentBranch: true,
          },
        }
      : undefined

  return (
    <>
      <output data-testid="route">
        {`${route.view}:${route.selectedId}:${String(route.conversationOpen)}`}
      </output>
      <button
        type="button"
        onClick={() => {
          setLatestHead(true)
          setRoute({
            view: 'board',
            selectedId: otherSession.id,
            conversationOpen: false,
          })
        }}
      >
        Open board
      </button>
      <button
        type="button"
        onClick={() => {
          feedback?.openActivity()
        }}
      >
        Open Activity
      </button>
      {currentSession === undefined ? null : (
        <ConversationPaneHeader
          session={currentSession}
          actions={actions}
          isMobile
          statusPanelOpen={false}
          backgroundTasks={[]}
          openInTerminalStatus={null}
          isToday={false}
          onAction={onAction}
          onOpenInTerminal={() => undefined}
          onSelectSession={() => undefined}
          onToggleStatusPanel={() => undefined}
        />
      )}
    </>
  )
}

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.clearAllMocks()
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

it('returns to the original conversation before confirming an Activity retry', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      Response.json(
        { error: 'PR #42 head changed since confirmation.' },
        { status: 409 },
      ),
    ),
  )
  vi.stubGlobal('fetch', fetchMock)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <SessionOperationProvider>
          <RetryNavigationHarness onAction={() => Promise.resolve(true)} />
        </SessionOperationProvider>
      </AsyncActionFeedbackProvider>,
    )
  })

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
    ).click()
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label="Merge PR #42"]',
      ),
    ).click()
  })
  await act(async () => {
    const confirmation = requireElement(
      container.querySelector<HTMLElement>('[role="alertdialog"]'),
    )
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--failure') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  act(() => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Open board',
      ) ?? null,
    ).click()
  })
  act(() => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Open Activity',
      ) ?? null,
    ).click()
  })
  await act(async () => {
    requireElement(
      [
        ...container.querySelectorAll<HTMLButtonElement>(
          '.async-action-activity-list button',
        ),
      ].find((button) => button.textContent === 'Retry') ?? null,
    ).click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    route: container.querySelector('[data-testid="route"]')?.textContent,
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    headSha: container.querySelector('.pull-request-merge-head-sha')
      ?.textContent,
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
    activityOpen: container.querySelector('[aria-label="Activity"]') !== null,
  })
  expect(snapshot()).toEqual({
    route: 'inbox:original-fixture-session:true',
    confirmationOpen: true,
    headSha: 'current-fixture-head',
    requestBodies: [
      JSON.stringify({
        owner: 'example-org',
        repo: 'example-repo',
        expectedHeadSha: 'old-fixture-head',
      }),
    ],
    activityOpen: false,
  })
})
