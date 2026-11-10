import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationPane } from '#components/conversation-pane'
import { ConversationPaneView } from '#components/conversation-pane-view'
import type { ChatItem, Session, SessionActions } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  kind: 'Question',
  title: 'Choose a retry policy',
  preview: null,
  elapsedSeconds: 0,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'service refresh',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const actions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
}

const question: ChatItem = {
  type: 'question',
  question: 'Which retry policy should be used?',
  choices: ['Retry a few times', 'Retry once'],
}

let container: HTMLDivElement
let root: Root | null = null
type ActionHandler = (
  action: string,
  payload?: Record<string, unknown>,
) => Promise<boolean>

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function renderView(
  onAction: ActionHandler,
  viewSession: Session = session,
  onSelectSession: (id: string) => void = () => undefined,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={viewSession}
        messages={[question]}
        actions={actions}
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
      />,
    )
  })
}

function dispatchKey(
  target: EventTarget,
  key: string,
  options: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
    ...options,
  })
  target.dispatchEvent(event)
  return event
}

function dispatchKeyInAct(
  target: EventTarget,
  key: string,
  options: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
    ...options,
  })
  act(() => {
    target.dispatchEvent(event)
  })
  return event
}

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

it('answers a question from a number key or a choice click', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)

  const keyEvent = dispatchKeyInAct(
    requireElement(container.querySelector('.conversation-pane')),
    '2',
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.chat-question-choices button')
      ?.click()
  })

  const actual = JSON.stringify([
    onAction.mock.calls,
    keyEvent.defaultPrevented,
  ])
  const expected = JSON.stringify([
    [
      ['answer', { choice: 2 }],
      ['answer', { choice: 1 }],
    ],
    true,
  ])
  expect(actual).toBe(expected)
})

it('sends on Enter and Command-Enter and preserves Shift-Enter', () => {
  const onAction = vi.fn(() => new Promise<boolean>(() => undefined))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )

  const changeText = (text: string) => {
    act(() => {
      const valueSetter = Reflect.getOwnPropertyDescriptor(
        textarea.constructor.prototype,
        'value',
      )?.set
      if (valueSetter !== undefined) {
        Reflect.apply(valueSetter, textarea, [text])
      }
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
    })
  }

  changeText('Use bounded retries')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  changeText('Keep the error visible')
  act(() => {
    dispatchKey(textarea, 'Enter', { metaKey: true })
  })
  changeText('Add one more detail')
  const shiftEvent = dispatchKeyInAct(textarea, 'Enter', { shiftKey: true })

  const actual = JSON.stringify([
    onAction.mock.calls,
    shiftEvent.defaultPrevented,
  ])
  const expected = JSON.stringify([
    [
      ['messages', { content: 'Use bounded retries' }],
      ['messages', { content: 'Keep the error visible' }],
    ],
    false,
  ])
  expect(actual).toBe(expected)
})

it('interrupts the session on Escape', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(container.querySelector('textarea'))
  const event = dispatchKeyInAct(textarea, 'Escape')

  const actual = JSON.stringify([onAction.mock.calls, event.defaultPrevented])
  const expected = JSON.stringify([[['interrupt']], true])
  expect(actual).toBe(expected)
})

it('does not use question shortcuts while typing', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(container.querySelector('textarea'))
  const event = dispatchKeyInAct(textarea, '1')

  const actual = JSON.stringify([onAction.mock.calls, event.defaultPrevented])
  const expected = JSON.stringify([[], false])
  expect(actual).toBe(expected)
})

it('selects the parent session from its link', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const onSelectSession = vi.fn()
  renderView(
    onAction,
    { ...session, parent: { id: 'parent-session', title: 'Parent session' } },
    onSelectSession,
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLAnchorElement>('.conversation-parent'),
    ).click()
  })

  expect(onSelectSession.mock.calls).toEqual([['parent-session']])
})

it('keeps the message draft when sending fails', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, ['Retain this draft'])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const actual = JSON.stringify([textarea.value, onAction.mock.calls])
  const expected = JSON.stringify([
    'Retain this draft',
    [['messages', { content: 'Retain this draft' }]],
  ])
  expect(actual).toBe(expected)
})

it('clears the message draft after a successful send', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, ['Send this message'])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const actual = JSON.stringify([textarea.value, onAction.mock.calls])
  const expected = JSON.stringify([
    '',
    [['messages', { content: 'Send this message' }]],
  ])
  expect(actual).toBe(expected)
})

it('shows the operation error returned by the server', async () => {
  const jsonResponse = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse([
        {
          type: 'permission',
          tool: 'Shell',
          description: 'Run the formatter on changed files.',
        },
      ]),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({ error: 'session pane is idle or unavailable' }, 409),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.permission-actions button'),
    ).click()
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const actionRequest = fetcher.mock.calls[2]?.[0]
  const actionRequestUrl =
    typeof actionRequest === 'string'
      ? actionRequest
      : actionRequest instanceof URL
        ? actionRequest.href
        : actionRequest?.url
  const actual = JSON.stringify([
    container.querySelector('[role="alert"]')?.textContent,
    actionRequestUrl,
  ])
  const expected = JSON.stringify([
    'session pane is idle or unavailable',
    '/api/sessions/sample-session/permission/approve',
  ])
  expect(actual).toBe(expected)
})

it('shows an HTTP status when the initial messages response has no body', async () => {
  const actionsResponse = new Response(
    JSON.stringify({
      message: false,
      answerQuestion: false,
      interrupt: false,
      openInTerminal: true,
    }),
    { headers: { 'Content-Type': 'application/json' } },
  )
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      new Response('', { status: 404, statusText: 'Not Found' }),
    )
    .mockResolvedValueOnce(actionsResponse)
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const actual = JSON.stringify([
    container.querySelector('[role="alert"]')?.textContent,
    container.querySelector('.conversation-status')?.textContent,
  ])
  const expected = JSON.stringify(['Not Found', 'No messages yet.'])
  expect(actual).toBe(expected)
})

it('reloads the selected conversation when the session feed changes', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse([{ type: 'assistant', text: 'Before update' }]),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse([{ type: 'assistant', text: 'After update' }]),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        interrupt: true,
        openInTerminal: true,
      }),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={1}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const actual = JSON.stringify([
    container.querySelector('.chat-assistant-message')?.textContent,
    fetcher.mock.calls.length,
  ])
  const expected = JSON.stringify(['After update', 4])
  expect(actual).toBe(expected)
})
