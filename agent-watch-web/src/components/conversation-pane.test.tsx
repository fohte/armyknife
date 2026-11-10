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

function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, [value])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function renderView(
  onAction: ActionHandler,
  viewSession: Session = session,
  onSelectSession: (id: string) => void = () => undefined,
  viewMessages: readonly ChatItem[] = [question],
  viewActions: SessionActions | null = actions,
  isMobile = false,
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={viewSession}
        messages={viewMessages}
        actions={viewActions}
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
        isMobile={isMobile}
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
  window.localStorage.clear()
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

it('sends on Enter and Command-Enter and preserves Shift-Enter', async () => {
  const completedSends: Array<(succeeded: boolean) => void> = []
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        completedSends.push(resolve)
      }),
  )
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )

  setTextareaValue(textarea, 'Use bounded retries')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    completedSends[0]?.(true)
    await Promise.resolve()
  })
  setTextareaValue(textarea, 'Keep the error visible')
  act(() => {
    dispatchKey(textarea, 'Enter', { metaKey: true })
  })
  await act(async () => {
    completedSends[1]?.(true)
    await Promise.resolve()
  })
  setTextareaValue(textarea, 'Add one more detail')
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

it('keeps Enter for newlines and exposes mobile conversation actions', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [question], actions, true)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )

  const enterEvent = dispatchKeyInAct(textarea, 'Enter')
  const shiftEnterEvent = dispatchKeyInAct(textarea, 'Enter', {
    shiftKey: true,
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.mobile-interrupt-button'),
    ).click()
  })

  const snapshot = () => ({
    actions: onAction.mock.calls,
    defaultPrevented: [
      enterEvent.defaultPrevented,
      shiftEnterEvent.defaultPrevented,
    ],
    keyboardHint: container.querySelector('.composer-footer > span')
      ?.textContent,
    choiceLabels: Array.from(
      container.querySelectorAll('.chat-question-choices kbd'),
      (element) => element.textContent,
    ),
    interruptLabel: container.querySelector('.mobile-interrupt-button')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    actions: [['interrupt']],
    defaultPrevented: [false, false],
    keyboardHint: undefined,
    choiceLabels: [],
    interruptLabel: 'Interrupt',
  })
})

it('does not send while Enter confirms an IME composition', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'A composed message')
  const composingEvent = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
    isComposing: true,
  })
  const legacyImeEvent = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  })
  Object.defineProperty(legacyImeEvent, 'keyCode', { value: 229 })
  act(() => {
    textarea.dispatchEvent(composingEvent)
    textarea.dispatchEvent(legacyImeEvent)
  })

  const snapshot = () => ({
    actions: onAction.mock.calls,
    defaultPrevented: [
      composingEvent.defaultPrevented,
      legacyImeEvent.defaultPrevented,
    ],
    draft: textarea.value,
  })
  expect(snapshot()).toEqual({
    actions: [],
    defaultPrevented: [false, false],
    draft: 'A composed message',
  })
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

it('opens and closes a crit review beside a waiting conversation', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, {
    ...session,
    category: 'needs_action',
    kind: 'Review',
    critUrl: 'http://127.0.0.1:5000/review/sample',
  })

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-crit-toggle'),
    ).click()
  })
  const openState = {
    expanded: requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-crit-toggle'),
    ).getAttribute('aria-expanded'),
    frameSource: requireElement(
      container.querySelector<HTMLIFrameElement>('.crit-pane iframe'),
    ).getAttribute('src'),
    closeLabel: requireElement(
      container.querySelector<HTMLButtonElement>('.crit-pane button'),
    ).getAttribute('aria-label'),
  }

  renderView(onAction, {
    ...session,
    category: 'needs_action',
    kind: 'Review',
    critUrl: 'http://127.0.0.1:5001/review/sample',
  })
  const changedUrlState = {
    expanded: requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-crit-toggle'),
    ).getAttribute('aria-expanded'),
    pane: container.querySelector('.crit-pane'),
  }
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-crit-toggle'),
    ).click()
  })

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.crit-pane button'),
    ).click()
  })
  const closedState = {
    expanded: requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-crit-toggle'),
    ).getAttribute('aria-expanded'),
    pane: container.querySelector('.crit-pane'),
  }

  const snapshot = () => ({ openState, changedUrlState, closedState })
  expect(snapshot()).toEqual({
    openState: {
      expanded: 'true',
      frameSource: 'http://127.0.0.1:5000/review/sample',
      closeLabel: 'Close crit',
    },
    changedUrlState: {
      expanded: 'false',
      pane: null,
    },
    closedState: {
      expanded: 'false',
      pane: null,
    },
  })
})

it('does not offer crit for sessions without a waiting crit URL', () => {
  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      category: 'needs_action',
      kind: 'Review',
      critUrl: null,
    },
  )
  const missingUrl = container.querySelector('.conversation-crit-toggle')

  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      category: 'done',
      kind: 'Review',
      critUrl: 'http://127.0.0.1:5000/review/sample',
    },
  )
  const notWaiting = container.querySelector('.conversation-crit-toggle')

  const snapshot = () => ({ missingUrl, notWaiting })
  expect(snapshot()).toEqual({
    missingUrl: null,
    notWaiting: null,
  })
})

it('directs Codex questions and permissions to the terminal', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const codexActions: SessionActions = {
    message: true,
    answerQuestion: false,
    interrupt: true,
    openInTerminal: true,
  }
  renderView(
    onAction,
    { ...session, engine: 'Codex' },
    undefined,
    [
      question,
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the project formatter.',
      },
    ],
    codexActions,
  )
  let shortcutEvent: KeyboardEvent | null = null
  act(() => {
    shortcutEvent = dispatchKey(window, '1')
  })
  const terminalButtons = Array.from(
    container.querySelectorAll<HTMLButtonElement>(
      '.conversation-terminal-fallback button',
    ),
  )
  act(() => {
    terminalButtons.forEach((button) => {
      button.click()
    })
  })

  const snapshot = () => ({
    disabledChoices: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    guidance: Array.from(
      container.querySelectorAll('.conversation-terminal-fallback span'),
      (element) => element.textContent,
    ),
    terminalButtons: terminalButtons.map((button) => button.textContent),
    composerDisabled: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).disabled,
    shortcutDefaultPrevented: shortcutEvent?.defaultPrevented,
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    disabledChoices: [true, true],
    guidance: [
      'Answer in terminal to continue.',
      'Review this permission in terminal.',
    ],
    terminalButtons: ['Open in terminal', 'Open in terminal'],
    composerDisabled: false,
    shortcutDefaultPrevented: false,
    actionCalls: [['focus'], ['focus']],
  })
})

it('keeps the message draft when sending fails', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Retain this draft')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: 'Retain this draft',
    storedDraft: 'Retain this draft',
    actions: [['messages', { content: 'Retain this draft' }]],
  })
})

it('clears the message draft after a successful send', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Send this message')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: '',
    storedDraft: null,
    actions: [['messages', { content: 'Send this message' }]],
  })
})

it('keeps text added after a send starts', async () => {
  let completeSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        completeSend = resolve
      }),
  )
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'First message')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  setTextareaValue(textarea, 'Next message')

  await act(async () => {
    completeSend?.(true)
    await Promise.resolve()
  })

  const snapshot = () => ({
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: 'Next message',
    storedDraft: 'Next message',
    actions: [['messages', { content: 'First message' }]],
  })
})

it('restores the message draft after the conversation is remounted', () => {
  renderView(() => Promise.resolve(true))
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Continue this draft')

  act(() => {
    root?.unmount()
  })
  root = null
  renderView(() => Promise.resolve(true))

  const snapshot = () => ({
    draft: container.querySelector<HTMLTextAreaElement>('textarea')?.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
  })
  expect(snapshot()).toEqual({
    draft: 'Continue this draft',
    storedDraft: 'Continue this draft',
  })
})

it('keeps the composer usable when browser storage is unavailable', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('storage unavailable')
  })
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('storage unavailable')
  })
  renderView(() => Promise.resolve(true))
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Keep editing in memory')

  const snapshot = () => ({
    draft: textarea.value,
    composerVisible: textarea.isConnected,
    storageNotice: container.querySelector('[role="status"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    draft: 'Keep editing in memory',
    composerVisible: true,
    storageNotice:
      'Browser storage is unavailable. Drafts may not survive a page reload.',
  })
})

it('keeps drafts separate for each session', () => {
  renderView(() => Promise.resolve(true))
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    'First session draft',
  )

  act(() => {
    root?.unmount()
  })
  root = null
  renderView(() => Promise.resolve(true), { ...session, id: 'another-session' })
  const otherTextarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(otherTextarea, 'Second session draft')

  act(() => {
    root?.unmount()
  })
  root = null
  renderView(() => Promise.resolve(true))

  const snapshot = () => ({
    selectedDraft:
      container.querySelector<HTMLTextAreaElement>('textarea')?.value,
    firstSessionDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    secondSessionDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:another-session',
    ),
  })
  expect(snapshot()).toEqual({
    selectedDraft: 'First session draft',
    firstSessionDraft: 'First session draft',
    secondSessionDraft: 'Second session draft',
  })
})

it('blocks duplicate sends and shows their pending status', async () => {
  let completeSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        completeSend = resolve
      }),
  )
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Send once')
  const events: KeyboardEvent[] = []
  act(() => {
    events.push(dispatchKey(textarea, 'Enter'))
    events.push(dispatchKey(textarea, 'Enter'))
  })
  const sendButton = requireElement(
    container.querySelector<HTMLButtonElement>('[aria-label="Send message"]'),
  )
  const duringSend = () => ({
    actionCalls: onAction.mock.calls,
    buttonDisabled: sendButton.disabled,
    defaultPrevented: events.map((event) => event.defaultPrevented),
    draft: textarea.value,
    status: container.querySelector('[role="status"]')?.textContent ?? null,
  })
  const pendingSnapshot = duringSend()

  await act(async () => {
    completeSend?.(true)
    await Promise.resolve()
  })

  const snapshot = () => ({
    duringSend: pendingSnapshot,
    afterSend: {
      buttonDisabled: sendButton.disabled,
      draft: textarea.value,
      status: container.querySelector('[role="status"]')?.textContent ?? null,
    },
  })
  const expected = {
    duringSend: {
      actionCalls: [['messages', { content: 'Send once' }]],
      buttonDisabled: true,
      defaultPrevented: [true, true],
      draft: 'Send once',
      status: 'Sending…',
    },
    afterSend: {
      buttonDisabled: true,
      draft: '',
      status: null,
    },
  }
  expect(snapshot()).toEqual(expected)
})

it('follows new messages at the bottom without moving a reader who scrolled up', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const firstMessage: ChatItem = {
    type: 'assistant',
    text: 'First message',
  }
  renderView(onAction, session, undefined, [firstMessage])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  let height = 300
  Object.defineProperties(scroll, {
    scrollHeight: { configurable: true, get: () => height },
    clientHeight: { configurable: true, value: 100 },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  renderView(onAction, session, undefined, [firstMessage])
  const initialBottom = scroll.scrollTop

  height = 500
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
  ])
  const afterNewMessage = scroll.scrollTop

  act(() => {
    scroll.scrollTop = 40
    scroll.dispatchEvent(new Event('scroll'))
  })
  height = 600
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
    { type: 'assistant', text: 'Third message' },
  ])
  const whileReadingEarlierMessages = scroll.scrollTop

  act(() => {
    scroll.scrollTop = 500
    scroll.dispatchEvent(new Event('scroll'))
  })
  height = 700
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
    { type: 'assistant', text: 'Third message' },
    { type: 'assistant', text: 'Fourth message' },
  ])

  const snapshot = () => ({
    initialBottom,
    afterNewMessage,
    whileReadingEarlierMessages,
    afterReturningToBottom: scroll.scrollTop,
  })
  expect(snapshot()).toEqual({
    initialBottom: 200,
    afterNewMessage: 400,
    whileReadingEarlierMessages: 40,
    afterReturningToBottom: 600,
  })
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

it('keeps the conversation and focused draft while a feed refresh is pending', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse([
        { type: 'assistant', text: 'Current conversation' },
        question,
        {
          type: 'permission',
          tool: 'Shell',
          description: 'Run the project formatter.',
        },
      ]),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: true,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockImplementation(() => new Promise<Response>(() => undefined))
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
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Draft during refresh')
  act(() => {
    textarea.focus()
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={1}
        onSelectSession={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    sameComposer: textarea === container.querySelector('textarea'),
    composerValue: textarea.value,
    composerDisabled: textarea.disabled,
    composerFocused: document.activeElement === textarea,
    transcript: container.querySelector('.chat-assistant-message')?.textContent,
    questionChoiceDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    permissionButtonsDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.permission-actions button',
      ),
      (button) => button.disabled,
    ),
    conversationStatus: container.querySelector('.conversation-status')
      ?.textContent,
    requestsStarted: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    sameComposer: true,
    composerValue: 'Draft during refresh',
    composerDisabled: false,
    composerFocused: true,
    transcript: 'Current conversation',
    questionChoiceDisabled: [false, false],
    permissionButtonsDisabled: [false, false],
    conversationStatus: undefined,
    requestsStarted: 4,
  })
})

it('keeps question and permission actions active after a refresh error', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse([
        question,
        {
          type: 'permission',
          tool: 'Shell',
          description: 'Run the project formatter.',
        },
      ]),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: true,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ error: 'Messages unavailable' }), {
        status: 503,
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: true,
        permission: { approve: true, reject: true },
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
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
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
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    questionChoiceDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    permissionButtonsDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.permission-actions button',
      ),
      (button) => button.disabled,
    ),
    error: container.querySelector('[role="alert"]')?.textContent,
    requestCount: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    questionChoiceDisabled: [false, false],
    permissionButtonsDisabled: [false, false],
    error: 'Messages unavailable',
    requestCount: 4,
  })
})
