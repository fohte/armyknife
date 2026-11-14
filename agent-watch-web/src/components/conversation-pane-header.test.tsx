import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationPaneHeader } from '#components/conversation-pane-header'
import type { Session, SessionActions } from '#types'

vi.mock('#components/copy-session-id-control', () => ({
  CopySessionIdControl: () => null,
}))

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'fixture-session',
  engine: 'claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Message',
  title: 'A fixture title',
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

function renderHeader(
  isMobile: boolean,
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>,
  currentSession = session,
) {
  act(() => {
    root?.render(
      <ConversationPaneHeader
        key={currentSession.id}
        session={currentSession}
        sessions={[]}
        actions={actions}
        isMobile={isMobile}
        statusPanelOpen={false}
        backgroundTasks={[]}
        openInTerminalStatus={null}
        isToday={false}
        onAction={onAction}
        onOpenInTerminal={() => undefined}
        onSelectSession={() => undefined}
        onToggleStatusPanel={() => undefined}
      />,
    )
  })
}

function setInputValue(input: HTMLInputElement, value: string) {
  const valueSetter = Reflect.getOwnPropertyDescriptor(
    input.constructor.prototype,
    'value',
  )?.set
  if (valueSetter !== undefined) Reflect.apply(valueSetter, input, [value])
  input.dispatchEvent(new Event('input', { bubbles: true }))
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

it.each([
  {
    description: 'desktop header saves a nonempty title',
    isMobile: false,
    title: 'Updated fixture title',
    saved: true,
    expectedDraft: null,
  },
  {
    description: 'mobile header clears the title',
    isMobile: true,
    title: '',
    saved: true,
    expectedDraft: null,
  },
  {
    description: 'desktop header keeps an unsaved title after failure',
    isMobile: false,
    title: 'Uncommitted fixture title',
    saved: false,
    expectedDraft: 'Uncommitted fixture title',
  },
])('$description', async ({ isMobile, title, saved, expectedDraft }) => {
  let completeSave: (result: boolean) => void = () => undefined
  const saveResult = new Promise<boolean>((resolve) => {
    completeSave = resolve
  })
  const onAction = vi.fn(
    (action: string, payload?: Record<string, unknown>): Promise<boolean> => {
      return action === 'title' && typeof payload?.['title'] === 'string'
        ? saveResult
        : Promise.resolve(false)
    },
  )
  renderHeader(isMobile, onAction)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Edit session title"]',
      ),
    ).click()
  })
  const input = requireElement(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Session title"]',
    ),
  )
  act(() => {
    setInputValue(input, title)
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('button[type="submit"]'),
    ).click()
  })
  const inputDisabledDuringSave = input.disabled
  const saveButtonTextDuringSave = requireElement(
    container.querySelector<HTMLButtonElement>('button[type="submit"]'),
  ).textContent
  await act(async () => {
    completeSave(saved)
    await saveResult
  })

  const snapshot = () => ({
    action: onAction.mock.calls,
    inputDisabledDuringSave,
    saveButtonTextDuringSave,
    editor: (() => {
      const editor = container.querySelector<HTMLInputElement>(
        'input[aria-label="Session title"]',
      )
      return editor === null
        ? { visible: false, draft: null }
        : { visible: true, draft: editor.value }
    })(),
  })
  expect(snapshot()).toEqual({
    action: [['title', { title }]],
    inputDisabledDuringSave: true,
    saveButtonTextDuringSave: 'Saving…',
    editor:
      expectedDraft === null
        ? { visible: false, draft: null }
        : { visible: true, draft: expectedDraft },
  })
})

it('starts a fresh editor when the selected session changes', () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderHeader(false, onAction)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Edit session title"]',
      ),
    ).click()
  })
  const input = requireElement(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Session title"]',
    ),
  )
  act(() => {
    setInputValue(input, 'Uncommitted fixture title')
  })

  renderHeader(false, onAction, {
    ...session,
    id: 'next-fixture-session',
    title: 'Another fixture title',
  })

  const snapshot = () => ({
    action: onAction.mock.calls,
    editButtonVisible:
      container.querySelector('[aria-label="Edit session title"]') !== null,
    editorVisible:
      container.querySelector('input[aria-label="Session title"]') !== null,
  })
  expect(snapshot()).toEqual({
    action: [],
    editButtonVisible: true,
    editorVisible: false,
  })
})

it('merges the session pull request after confirming its head SHA', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(Response.json({ result: 'merged' })),
  )
  vi.stubGlobal('fetch', fetchMock)
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'sample-head-sha',
      },
    },
  )

  act(() => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Merge',
      ) ?? null,
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  const confirmedSha = confirmation.querySelector('code')?.textContent
  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="status"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    confirmedSha,
    request: fetchMock.mock.calls.map(([url, init]) => ({
      url,
      method: init?.method,
      body: init?.body,
    })),
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    mergeButton:
      container.querySelector('.conversation-pr-merge-button') !== null,
    feedback: container.querySelector('[role="status"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    confirmedSha: 'sample-head-sha',
    request: [
      {
        url: '/api/sessions/fixture-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'sample-head-sha',
        }),
      },
    ],
    confirmationOpen: false,
    mergeButton: false,
    feedback: 'PR #42 merged.',
  })
})

it('hides the merge action when the session pull request has no head SHA', () => {
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        headSha: null,
      },
    },
  )

  const headerState = () => ({
    pullRequestLink: container.querySelector('.conversation-pr-link')
      ?.textContent,
    mergeButton: [
      ...container.querySelectorAll<HTMLButtonElement>('button'),
    ].some((button) => button.textContent === 'Merge'),
  })
  expect(headerState()).toEqual({
    pullRequestLink: '#42 PR',
    mergeButton: false,
  })
})
