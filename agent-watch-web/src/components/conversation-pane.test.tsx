import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationPane } from '#components/conversation-pane'
import { ConversationPaneView } from '#components/conversation-pane-view'
import type { BackgroundTask, ChatItem, Session, SessionActions } from '#types'

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  isPaused: false,
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
  answer: null,
}

const proposal: ChatItem = {
  type: 'proposal',
  seq: 3,
  title: 'When should the setting take effect?',
  summary: 'Choose how the change is introduced.',
  options: [
    {
      label: 'Use the existing flow',
      description: 'Explain the change before applying it.',
      pros: ['Small change'],
      cons: ['Adds one step'],
      recommended: true,
    },
    {
      label: 'Show the choice earlier',
      description: null,
      pros: [],
      cons: [],
      recommended: false,
    },
  ],
  answer: null,
  resolved: false,
}

function messagesResponse(messages: unknown, backgroundTasks: unknown = []) {
  return { messages, backgroundTasks }
}

const permission: ChatItem = {
  type: 'permission',
  tool: 'Shell',
  description: 'Run the formatter on changed files.',
}

const otherPermission: ChatItem = {
  type: 'permission',
  tool: 'Subagent sample-agent',
  description: 'Waiting for permission approval.',
}
let container: HTMLDivElement
let root: Root | null = null
const originalExecCommand = Object.getOwnPropertyDescriptor(
  document,
  'execCommand',
)
const originalClipboard = Object.getOwnPropertyDescriptor(
  navigator,
  'clipboard',
)
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
  viewBackgroundTasks: readonly BackgroundTask[] = [],
  viewSessions: readonly Session[] = [],
  statusPanelOpen = false,
  onToggleStatusPanel?: () => void,
  pendingPermissionAction?: {
    action: 'approve' | 'reject'
    status: 'sending' | 'sent'
    tool: string
    description: string
  } | null,
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={viewSession}
        sessions={viewSessions}
        messages={viewMessages}
        backgroundTasks={viewBackgroundTasks}
        actions={viewActions}
        pendingPermissionAction={pendingPermissionAction}
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
        isMobile={isMobile}
        statusPanelOpen={statusPanelOpen}
        {...(onToggleStatusPanel === undefined ? {} : { onToggleStatusPanel })}
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
  if (originalExecCommand === undefined) {
    Reflect.deleteProperty(document, 'execCommand')
  } else {
    Object.defineProperty(document, 'execCommand', originalExecCommand)
  }
  if (originalClipboard === undefined) {
    Reflect.deleteProperty(navigator, 'clipboard')
  } else {
    Object.defineProperty(navigator, 'clipboard', originalClipboard)
  }
})

it('shows pending and sent permission action feedback', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const renderPermission = (status: 'sending' | 'sent') => {
    renderView(
      onAction,
      session,
      undefined,
      [permission, otherPermission],
      actions,
      false,
      [],
      [],
      false,
      undefined,
      {
        action: 'approve',
        status,
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    )
  }
  renderPermission('sending')
  const cards = () =>
    Array.from(container.querySelectorAll('.chat-permission-card'))
  const sending = {
    cards: cards().map((card) => ({
      labels: Array.from(card.querySelectorAll('button')).map(
        (button) => button.textContent,
      ),
      disabled: Array.from(
        card.querySelectorAll<HTMLButtonElement>('button'),
      ).map((button) => button.disabled),
      status: card.querySelector('[role="status"]')?.textContent ?? null,
    })),
  }

  renderPermission('sent')

  const snapshot = () => ({
    sending,
    sent: {
      cards: cards().map((card) => ({
        labels: Array.from(card.querySelectorAll('button')).map(
          (button) => button.textContent,
        ),
        disabled: Array.from(
          card.querySelectorAll<HTMLButtonElement>('button'),
        ).map((button) => button.disabled),
        status: card.querySelector('[role="status"]')?.textContent ?? null,
      })),
    },
  })
  expect(snapshot()).toEqual({
    sending: {
      cards: [
        {
          labels: ['Approving…', 'Reject'],
          disabled: [true, true],
          status: null,
        },
        {
          labels: ['Approve', 'Reject'],
          disabled: [false, false],
          status: null,
        },
      ],
    },
    sent: {
      cards: [
        {
          labels: ['Approve', 'Reject'],
          disabled: [true, true],
          status: 'Approval sent. Session is resuming…',
        },
        {
          labels: ['Approve', 'Reject'],
          disabled: [false, false],
          status: null,
        },
      ],
    },
  })
})

it('copies the session ID from desktop and mobile headers', async () => {
  let copiedText = ''
  const writeText = vi.fn((text: string) => {
    copiedText = text
    return Promise.resolve()
  })
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)

  const desktopButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-header-actions .conversation-copy-session-id',
    ),
  )
  await act(async () => {
    desktopButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  const desktopButtonText = desktopButton.textContent

  renderView(onAction, session, undefined, [question], actions, true)
  const mobileButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    mobileButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copiedText,
    writeCalls: writeText.mock.calls,
    desktopButtonText,
    mobileButtonText: mobileButton.textContent,
    mobileAccessibleName: mobileButton.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    copiedText: session.id,
    writeCalls: [[session.id], [session.id]],
    desktopButtonText: 'Copied',
    mobileButtonText: 'Copied',
    mobileAccessibleName: 'Session ID copied',
  })
})

it('copies the session ID through the HTTP-compatible clipboard fallback', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  let copiedText = ''
  const copyCommand = vi.fn(() => {
    copiedText =
      Array.from(document.body.querySelectorAll('textarea')).at(-1)?.value ?? ''
    return true
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: copyCommand,
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [question],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  let copyCallsAtClickReturn = 0
  act(() => {
    copyButton.click()
    copyCallsAtClickReturn = copyCommand.mock.calls.length
  })
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copiedText,
    copyCalls: copyCommand.mock.calls,
    copyCallsAtClickReturn,
    temporaryTextareaCount: document.body.querySelectorAll('textarea').length,
    buttonText: copyButton.textContent,
  })
  expect(snapshot()).toEqual({
    copiedText: session.id,
    copyCalls: [['copy']],
    copyCallsAtClickReturn: 1,
    temporaryTextareaCount: 1,
    buttonText: 'Copied',
  })
})

it('falls back when the Clipboard API rejects the copy request', async () => {
  const writeText = vi.fn(() => Promise.reject(new Error('copy denied')))
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })
  let copiedText = ''
  const copyCommand = vi.fn(() => {
    copiedText =
      Array.from(document.body.querySelectorAll('textarea')).at(-1)?.value ?? ''
    return true
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: copyCommand,
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [question],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    copyButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    clipboardCalls: writeText.mock.calls,
    fallbackCalls: copyCommand.mock.calls,
    copiedText,
    buttonText: copyButton.textContent,
  })
  expect(snapshot()).toEqual({
    clipboardCalls: [[session.id]],
    fallbackCalls: [['copy']],
    copiedText: session.id,
    buttonText: 'Copied',
  })
})

it('announces when neither clipboard method can copy the session ID', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: vi.fn(() => false),
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [question],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    copyButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    buttonText: copyButton.textContent,
    accessibleName: copyButton.getAttribute('aria-label'),
    liveStatus: container.querySelector('[role="status"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    buttonText: 'Copy failed',
    accessibleName: 'Could not copy session ID',
    liveStatus: 'Could not copy session ID',
  })
})

it('announces copy failure and removes the textarea when execCommand throws', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  const copyCommand = vi.fn(() => {
    throw new Error('copy unavailable')
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: copyCommand,
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [question],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    copyButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copyCalls: copyCommand.mock.calls,
    temporaryTextareaCount: document.body.querySelectorAll('textarea').length,
    buttonText: copyButton.textContent,
    liveStatus: container.querySelector('[role="status"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    copyCalls: [['copy']],
    temporaryTextareaCount: 1,
    buttonText: 'Copy failed',
    liveStatus: 'Could not copy session ID',
  })
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

it('keeps the message composer available while a proposal waits for a choice', () => {
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [proposal],
  )

  const snapshot = () => ({
    placeholder: container
      .querySelector<HTMLTextAreaElement>('.conversation-composer textarea')
      ?.getAttribute('placeholder'),
    composerDisabled: container.querySelector<HTMLTextAreaElement>(
      '.conversation-composer textarea',
    )?.disabled,
  })

  expect(snapshot()).toEqual({
    placeholder: 'Choose an option above, or message the session',
    composerDisabled: false,
  })
})

it('sends Command-Enter from the message composer without answering the proposal', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [proposal])
  act(() => {
    requireElement(
      container.querySelector<HTMLInputElement>('.chat-proposal-option input'),
    ).click()
  })
  const messageField = requireElement(
    container.querySelector<HTMLTextAreaElement>(
      '.conversation-composer textarea',
    ),
  )
  setTextareaValue(messageField, 'Send a direct message.')
  const submitEvent = dispatchKeyInAct(messageField, 'Enter', { metaKey: true })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    submitPrevented: submitEvent.defaultPrevented,
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    submitPrevented: true,
    actionCalls: [['messages', { content: 'Send a direct message.' }]],
  })
})

it('shows a selected answer, dims other choices, and renders Markdown', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    {
      ...question,
      question: 'Which **retry** policy should be used?',
      choices: ['Retry *a few* times', 'Retry once'],
      answer: 'Retry once',
    },
  ])
  dispatchKeyInAct(window, '1')
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.chat-question-choices button')
      ?.click()
  })

  const card = requireElement(container.querySelector('.chat-question-card'))
  const snapshot = () => ({
    question: card.querySelector('.chat-question-text strong')?.textContent,
    markdownChoice: card.querySelector('.chat-question-choice-text em')
      ?.textContent,
    answer: card.querySelector('.chat-question-answer')?.textContent,
    choices: Array.from(
      card.querySelectorAll<HTMLButtonElement>('.chat-question-choices button'),
      (button) => ({
        label: button.querySelector('.chat-question-choice-text')?.textContent,
        disabled: button.disabled,
        selected: button.classList.contains('is-selected'),
        dimmed: button.classList.contains('is-unselected'),
        check: button.querySelector('.chat-question-choice-check')?.textContent,
      }),
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    question: 'retry',
    markdownChoice: 'a few',
    answer: undefined,
    choices: [
      {
        label: 'Retry a few times',
        disabled: true,
        selected: false,
        dimmed: true,
        check: undefined,
      },
      {
        label: 'Retry once',
        disabled: true,
        selected: true,
        dimmed: false,
        check: '✓',
      },
    ],
    actionCalls: [],
  })
})

it('shows a freeform answer below the choices', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    { ...question, answer: 'Wait until the next deployment' },
  ])

  const snapshot = () => ({
    answer: container.querySelector('.chat-question-answer')?.textContent,
    selectedChoices: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button.is-selected',
      ),
      (button) => button.textContent,
    ),
    disabledChoices: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    answer: 'Answer: Wait until the next deployment',
    selectedChoices: [],
    disabledChoices: [true, true],
    actionCalls: [],
  })
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
  const onSelectSession = vi.fn<(id: string) => void>()
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

it('links known session IDs in user, rich user, and assistant messages', () => {
  const linkedSession: Session = {
    ...session,
    id: '11111111-1111-4111-8111-111111111111',
    category: 'done',
    title: 'Inspect the retry path',
    project: 'example toolkit',
  }
  const onSelectSession = vi.fn<(id: string) => void>()
  renderView(
    vi.fn().mockResolvedValue(true),
    session,
    onSelectSession,
    [
      { type: 'user', text: `Use ${linkedSession.id}` },
      {
        type: 'user_with_content',
        content: [{ type: 'text', text: `Inspect ${linkedSession.id}` }],
      },
      { type: 'assistant', text: `I found ${linkedSession.id}` },
    ],
    actions,
    false,
    [],
    [linkedSession],
  )

  const links = Array.from(
    container.querySelectorAll<HTMLAnchorElement>(
      '.chat-user-message a, .chat-assistant-message a',
    ),
  )
  for (const link of links) {
    act(() => {
      link.click()
    })
  }

  const expected = {
    messageClasses: [
      'chat-user-group',
      'chat-user-group',
      'chat-assistant-message',
    ],
    linkHrefs: [
      `?session=${linkedSession.id}`,
      `?session=${linkedSession.id}`,
      `?session=${linkedSession.id}`,
    ],
    selectedIds: [linkedSession.id, linkedSession.id, linkedSession.id],
  }
  const getActual = () => ({
    messageClasses: links.map((link) => link.closest('article')?.className),
    linkHrefs: links.map((link) => link.getAttribute('href')),
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  expect(getActual()).toEqual(expected)
})

it('marks a queued prompt below the delivered conversation', () => {
  renderView(vi.fn().mockResolvedValue(true), session, undefined, [
    { type: 'user', text: 'Please summarize the current changes.' },
    { type: 'assistant', text: 'I am reviewing the current changes.' },
    {
      type: 'queued_user',
      content: [{ type: 'text', text: 'Also check the error handling.' }],
    },
  ])

  const actual = Array.from(
    container.querySelectorAll<HTMLElement>(
      '.chat-user-group, .chat-assistant-message',
    ),
  ).map((message) => ({
    className: message.className,
    label: message.querySelector('.chat-queued-label')?.textContent ?? null,
    text: message.querySelector('.markdown-message')?.textContent ?? null,
    bubbleClassName:
      message.querySelector('.chat-user-message')?.className ?? null,
  }))

  expect(actual).toEqual([
    {
      className: 'chat-user-group',
      label: null,
      text: 'Please summarize the current changes.',
      bubbleClassName: 'markdown-message chat-user-message',
    },
    {
      className: 'chat-assistant-message',
      label: null,
      text: 'I am reviewing the current changes.',
      bubbleClassName: null,
    },
    {
      className: 'chat-user-group chat-user-group-queued',
      label: 'Queued',
      text: 'Also check the error handling.',
      bubbleClassName:
        'markdown-message chat-user-message chat-user-message-queued',
    },
  ])
})

it('shows peer images and supports keyboard navigation', () => {
  const linkedSession: Session = {
    ...session,
    id: 'fictional-peer-session',
    category: 'in_progress',
    title: 'Build verification',
  }
  const onSelectSession = vi.fn<(id: string) => void>()
  renderView(
    vi.fn().mockResolvedValue(true),
    session,
    onSelectSession,
    [
      {
        type: 'peer',
        direction: 'to',
        session_id: linkedSession.id,
        content: [
          { type: 'text', text: 'Please check the fictional output.' },
          {
            type: 'image',
            source: '/api/images/fixture-peer-image.png',
            available: false,
          },
        ],
      },
      {
        type: 'peer',
        direction: 'from',
        session_id: linkedSession.id,
        content: [
          { type: 'text', text: 'The fictional output passed.' },
          {
            type: 'image',
            source: '/api/images/fixture-peer-image.png',
            available: true,
          },
          { type: 'file', name: 'fixture-peer-layout.html' },
        ],
      },
      {
        type: 'event',
        kind: 'hook',
        text: 'A fictional hook completed.',
        failed: false,
      },
      {
        type: 'event',
        kind: 'system',
        text: 'A fictional operation was blocked.',
        failed: true,
      },
    ],
    actions,
    false,
    [],
    [linkedSession],
  )

  const peerLink = requireElement(
    container.querySelector<HTMLAnchorElement>(
      '.chat-peer-message-from .session-link-trigger',
    ),
  )
  const fromContent = requireElement(
    container.querySelector<HTMLElement>(
      '.chat-peer-message-from .chat-peer-content',
    ),
  )
  const toContent = requireElement(
    container.querySelector<HTMLElement>(
      '.chat-peer-message-to .chat-peer-content',
    ),
  )
  act(() => {
    toContent.click()
  })
  dispatchKeyInAct(fromContent, 'Enter')
  dispatchKeyInAct(toContent, ' ')

  const snapshot = () => ({
    sessionLinkHref: peerLink.getAttribute('href'),
    items: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.chat-peer-message, .chat-event',
      ),
      (element) => ({
        className: element.className,
        text: element.textContent,
        failed: element.getAttribute('data-failed'),
        kind: element.getAttribute('data-kind'),
        imageSources: Array.from(
          element.querySelectorAll<HTMLImageElement>('img'),
          (image) => image.getAttribute('src'),
        ),
        unavailableImageText: element.querySelector(
          '.chat-peer-image-unavailable',
        )?.textContent,
        fileNames: Array.from(
          element.querySelectorAll<HTMLElement>('.chat-peer-file'),
          (file) => file.textContent,
        ),
      }),
    ),
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  const expected = {
    sessionLinkHref: `?session=${linkedSession.id}`,
    items: [
      {
        className: 'chat-peer-message chat-peer-message-to',
        text: 'TOBuild verificationPlease check the fictional output.画像はもうありません',
        failed: null,
        kind: null,
        imageSources: [],
        unavailableImageText: '画像はもうありません',
        fileNames: [],
      },
      {
        className: 'chat-peer-message chat-peer-message-from',
        text: 'FROMBuild verificationThe fictional output passed.fixture-peer-layout.html',
        failed: null,
        kind: null,
        imageSources: ['/api/images/fixture-peer-image.png'],
        unavailableImageText: undefined,
        fileNames: ['fixture-peer-layout.html'],
      },
      {
        className: 'chat-event',
        text: 'A fictional hook completed.',
        failed: null,
        kind: 'hook',
        imageSources: [],
        unavailableImageText: undefined,
        fileNames: [],
      },
      {
        className: 'chat-event',
        text: 'A fictional operation was blocked.',
        failed: 'true',
        kind: 'system',
        imageSources: [],
        unavailableImageText: undefined,
        fileNames: [],
      },
    ],
    selectedIds: [linkedSession.id, linkedSession.id, linkedSession.id],
  }
  expect(snapshot()).toEqual(expected)
})

it('refreshes peer session labels when the linked session title changes', () => {
  const linkedSession: Session = {
    ...session,
    id: 'fictional-peer-session',
    title: 'Earlier task name',
  }
  const message: ChatItem = {
    type: 'peer',
    direction: 'from',
    session_id: linkedSession.id,
    content: [{ type: 'text', text: 'A fictional update.' }],
  }
  const onAction = vi.fn().mockResolvedValue(true)
  renderView(
    onAction,
    session,
    undefined,
    [message],
    actions,
    false,
    [],
    [linkedSession],
  )
  const labelBeforeRefresh = requireElement(
    container.querySelector<HTMLElement>('.chat-peer-label'),
  ).textContent

  renderView(
    onAction,
    session,
    undefined,
    [message],
    actions,
    false,
    [],
    [{ ...linkedSession, title: 'Updated task name' }],
  )

  const snapshot = () => ({
    labelBeforeRefresh,
    labelAfterRefresh: container.querySelector('.chat-peer-label')?.textContent,
  })
  expect(snapshot()).toEqual({
    labelBeforeRefresh: 'FROMEarlier task name',
    labelAfterRefresh: 'FROMUpdated task name',
  })
})

it('renders file attachments returned by the messages endpoint', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          {
            type: 'user_with_content',
            content: [
              { type: 'text', text: 'Inspect this document.' },
              { type: 'file', name: 'sample-layout.html' },
            ],
          },
        ]),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
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
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    message: container.querySelector('.chat-user-message')?.textContent,
    attachment: container.querySelector('.chat-user-file')?.textContent,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    message: 'Inspect this document.',
    attachment: 'sample-layout.html',
    alert: null,
  })
})

it('posts a proposal answer to its sequence route and refreshes the conversation', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const requests: Array<{
    url: string
    method: string
    body: string | null
  }> = []
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url =
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.toString()
          : input.url
    const method = init?.method ?? 'GET'
    requests.push({
      url,
      method,
      body: typeof init?.body === 'string' ? init.body : null,
    })
    return Promise.resolve(
      method === 'POST'
        ? new Response(null, { status: 204 })
        : url.endsWith('/messages')
          ? jsonResponse(messagesResponse([proposal]))
          : jsonResponse(actions),
    )
  })
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
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  act(() => {
    requireElement(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      )[1] ?? null,
    ).click()
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-proposal-actions .aw-button--primary',
      ),
    ).click()
  })
  await act(async () => {
    for (let index = 0; index < 8; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    post: requests.find((request) => request.method === 'POST') ?? null,
    placeholder: container
      .querySelector<HTMLTextAreaElement>('.conversation-composer textarea')
      ?.getAttribute('placeholder'),
    status: container.querySelector('.chat-proposal-card h2')?.textContent,
  })
  expect(snapshot()).toEqual({
    post: {
      url: '/api/sessions/sample-session/proposals/3/answer',
      method: 'POST',
      body: '{"choice":1,"comment":null}',
    },
    placeholder: 'Choose an option above, or message the session',
    status: 'Proposal · chose B',
  })
})
it('shows elapsed time while the session is working without an active tool', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'in_progress', elapsedSeconds: 92 },
    undefined,
    [],
  )

  const snapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status')
      ?.textContent,
    message: container.querySelector('.conversation-status')?.textContent,
  })

  expect(snapshot()).toEqual({
    workingStatus: 'Working · 1m 32s',
    message: 'No messages yet.',
  })
})

it('expands the background task count into descriptions and elapsed times', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'in_progress', elapsedSeconds: 92 },
    undefined,
    [],
    actions,
    false,
    [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
      },
      {
        id: 'fixture-task-two',
        description: 'Review the generated report',
        elapsedSeconds: 43,
      },
    ],
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLElement>(
        '.conversation-background-tasks summary',
      ),
    ).click()
  })

  const snapshot = () => ({
    summary: container.querySelector('.conversation-background-tasks summary')
      ?.textContent,
    tasks: Array.from(
      container.querySelectorAll('.conversation-background-tasks li'),
      (item) => item.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    summary: 'Background tasks (2)',
    tasks: ['Watch the build pipeline2m 8s', 'Review the generated report43s'],
  })
})

it('shows the compacting status while hiding elapsed time and stale tool details', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, isCompacting: true, elapsedSeconds: 92 },
    undefined,
    [
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo test --manifest-path fixture/Cargo.toml',
      },
    ],
  )

  const getSnapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status')
      ?.textContent,
    toolItems: container.querySelectorAll('.conversation-working-tool').length,
    inProgressItems: container.querySelectorAll('.tool-in-progress').length,
  })
  expect(getSnapshot()).toEqual({
    workingStatus: 'Compacting conversation…',
    toolItems: 0,
    inProgressItems: 0,
  })
})

it('shows an active tool and hides it from the completed conversation', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'in_progress', elapsedSeconds: 92 },
    undefined,
    [
      {
        type: 'tool_summary',
        summary: '1 action',
      },
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo test --manifest-path fixture/Cargo.toml',
      },
    ],
    actions,
    true,
  )

  const snapshot = () => ({
    workingStatus: Array.from(
      requireElement(container.querySelector('.conversation-working-status'))
        .children,
    )
      .slice(1)
      .map((child) => child.textContent),
    toolSummary:
      container.querySelector('.chat-tool-summary')?.textContent ?? null,
    inProgressItems: container.querySelectorAll('.tool-in-progress').length,
  })

  expect(snapshot()).toEqual({
    workingStatus: [
      'Working · 1m 32s',
      'execcargo test --manifest-path fixture/Cargo.toml',
    ],
    toolSummary: '1 action',
    inProgressItems: 0,
  })
})

it('hides working status when the session is done', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', elapsedSeconds: 92 },
    undefined,
    [
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo test',
      },
    ],
  )

  const snapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status'),
    toolSummary:
      container.querySelector('.chat-tool-summary summary')?.textContent ??
      null,
    operations: Array.from(
      container.querySelectorAll('.chat-tool-operations li'),
    ).map((operation) => operation.textContent),
  })

  expect(snapshot()).toEqual({
    workingStatus: null,
    toolSummary: '1 action',
    operations: ['execcargo test'],
  })
})

it('keeps background tasks visible while the session waits for input', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'needs_action' },
    undefined,
    [],
    actions,
    false,
    [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
      },
    ],
  )

  const snapshot = () => ({
    status: container.querySelector('.conversation-working-status')
      ?.textContent,
    workingDotVisible:
      container.querySelector('.conversation-working-dot') !== null,
    backgroundTasks: container.querySelector(
      '.conversation-background-tasks summary',
    )?.textContent,
  })

  expect(snapshot()).toEqual({
    status: 'Background tasks (1)Watch the build pipeline2m 8s',
    workingDotVisible: false,
    backgroundTasks: 'Background tasks (1)',
  })
})

it('accepts classified messages and the active tool from messages endpoint', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(
          [
            {
              type: 'peer',
              direction: 'from',
              session_id: 'fixture-peer-session',
              session_title: 'Fixture peer session',
              content: [
                { type: 'text', text: 'A fictional peer update.' },
                {
                  type: 'image',
                  source: '/api/images/fixture-peer-image.png',
                  available: true,
                },
                { type: 'file', name: 'fixture-peer-layout.html' },
              ],
            },
            {
              type: 'event',
              kind: 'armyknife',
              text: 'A fictional event was blocked.',
              failed: true,
            },
            {
              type: 'tool_in_progress',
              tool: 'exec',
              target: 'cargo test',
            },
          ],
          [
            {
              id: 'fixture-task-one',
              description: 'Watch the build pipeline',
              elapsedSeconds: 128,
            },
          ],
        ),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: false,
        answerQuestion: false,
        interrupt: false,
        openInTerminal: false,
      }),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, category: 'in_progress', elapsedSeconds: 30 }}
        refreshKey={0}
        onSelectSession={() => undefined}
        isMobile
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status')
      ?.children[1]?.textContent,
    activeTool: container.querySelector('.conversation-working-tool')
      ?.textContent,
    peerMessage: container.querySelector('.chat-peer-message')?.textContent,
    peerImage: container
      .querySelector<HTMLImageElement>('.chat-peer-message img')
      ?.getAttribute('src'),
    peerFile: container.querySelector('.chat-peer-file')?.textContent,
    event: Array.from(
      container.querySelectorAll<HTMLElement>('.chat-event'),
      (element) => ({
        text: element.textContent,
        failed: element.getAttribute('data-failed'),
      }),
    ),
    backgroundTasks: container.querySelector(
      '.conversation-background-tasks summary',
    )?.textContent,
  })
  expect(snapshot()).toEqual({
    workingStatus: 'Working · 30s',
    activeTool: 'execcargo test',
    peerMessage:
      'FROMFixture peer session (fixture-peer-session)A fictional peer update.fixture-peer-layout.html',
    peerImage: '/api/images/fixture-peer-image.png',
    peerFile: 'fixture-peer-layout.html',
    event: [{ text: 'A fictional event was blocked.', failed: 'true' }],
    backgroundTasks: 'Background tasks (1)',
  })
})

it('shows the status panel toggle and reports its open state', () => {
  const onToggleStatusPanel = vi.fn<() => void>()
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    () => undefined,
    [question],
    actions,
    false,
    [],
    [],
    true,
    onToggleStatusPanel,
  )
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-status-panel-toggle',
    ),
  )
  act(() => {
    toggle.click()
  })

  const snapshot = () => ({
    expanded: toggle.getAttribute('aria-expanded'),
    controls: toggle.getAttribute('aria-controls'),
    calls: onToggleStatusPanel.mock.calls,
  })
  expect(snapshot()).toEqual({
    expanded: 'true',
    controls: 'session-status-panel',
    calls: [[]],
  })
})

it('shows the paused state in the conversation header', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', kind: null, isPaused: true },
    undefined,
    [],
    null,
  )

  const snapshot = () => ({
    title: container.querySelector('.conversation-header h1')?.textContent,
    paused: container.querySelector('.conversation-header [data-slot="chip"]')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    title: 'Choose a retry policy',
    paused: 'Paused',
  })
})

it('shows the paused state in the mobile conversation header', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', kind: null, isPaused: true },
    undefined,
    [],
    null,
    true,
  )

  const snapshot = () => ({
    mobile: container
      .querySelector('.conversation-pane')
      ?.getAttribute('data-mobile'),
    title: container.querySelector('.mobile-conversation-heading h1')
      ?.textContent,
    paused: container.querySelector(
      '.mobile-conversation-heading [data-slot="chip"]',
    )?.textContent,
  })
  expect(snapshot()).toEqual({
    mobile: 'true',
    title: 'Choose a retry policy',
    paused: 'Paused',
  })
})

it('keeps the status panel toggle available without a waiting crit URL', () => {
  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      category: 'needs_action',
      kind: 'Review',
      critUrl: null,
    },
  )
  const missingUrl =
    container.querySelector('.conversation-status-panel-toggle') !== null

  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      category: 'done',
      kind: 'Review',
      critUrl: 'http://127.0.0.1:5000/review/sample',
    },
  )
  const notWaiting =
    container.querySelector('.conversation-status-panel-toggle') !== null

  const snapshot = () => ({ missingUrl, notWaiting })
  expect(snapshot()).toEqual({
    missingUrl: true,
    notWaiting: true,
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

it('sends pasted file data with the message and keeps it when sending fails', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const file = new File(['payload'], 'clipboard.bin', {
    type: 'application/octet-stream',
  })
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })

  const snapshot = () => ({
    actionCalls: onAction.mock.calls,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'clipboard.bin',
              mimeType: 'application/octet-stream',
              size: 7,
              data: 'cGF5bG9hZA==',
            },
          ],
        },
      ],
    ],
    attachmentNames: ['clipboard.bin'],
  })
})

it('attaches dropped files through the composer and sends their data', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const file = new File(['payload'], 'dropped.png', { type: 'image/png' })
  const pane = requireElement(container.querySelector('.conversation-pane'))
  const dataTransfer = { types: ['Files'], files: [file], dropEffect: 'none' }
  const dragEnter = new Event('dragenter', { bubbles: true, cancelable: true })
  Object.defineProperty(dragEnter, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(dragEnter)
  })
  const targetVisibleDuringDrag = container.querySelector(
    '.conversation-drop-target',
  )?.textContent
  const drop = new Event('drop', { bubbles: true, cancelable: true })
  Object.defineProperty(drop, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(drop)
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
  })

  const snapshot = () => ({
    targetVisibleDuringDrag,
    targetAfterDrop:
      container.querySelector('.conversation-drop-target')?.textContent ?? null,
    dropPrevented: drop.defaultPrevented,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    targetVisibleDuringDrag: 'Drop files to attach',
    targetAfterDrop: null,
    dropPrevented: true,
    attachmentNames: [],
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'dropped.png',
              mimeType: 'image/png',
              size: 7,
              data: 'cGF5bG9hZA==',
            },
          ],
        },
      ],
    ],
  })
})

it('hides the drop target when a file drag leaves the conversation', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const pane = requireElement(container.querySelector('.conversation-pane'))
  const dataTransfer = { types: ['Files'], files: [] }
  const dragEnter = new Event('dragenter', { bubbles: true, cancelable: true })
  Object.defineProperty(dragEnter, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(dragEnter)
  })
  const targetAfterEnter = container.querySelector(
    '.conversation-drop-target',
  )?.textContent
  const dragLeave = new Event('dragleave', { bubbles: true, cancelable: true })
  Object.defineProperty(dragLeave, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(dragLeave)
  })

  const snapshot = () => ({
    targetAfterEnter,
    targetAfterLeave:
      container.querySelector('.conversation-drop-target')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    targetAfterEnter: 'Drop files to attach',
    targetAfterLeave: null,
  })
})

it('clears pasted attachments after a successful send', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const file = new File(['payload'], 'clipboard.bin', {
    type: 'application/octet-stream',
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    actionCalls: onAction.mock.calls,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'clipboard.bin',
              mimeType: 'application/octet-stream',
              size: 7,
              data: 'cGF5bG9hZA==',
            },
          ],
        },
      ],
    ],
    attachmentNames: [],
  })
})

it('names the attachment that crosses the message size limit before sending', () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const firstFile = new File(['first'], 'first.bin', {
    type: 'application/octet-stream',
  })
  const limitCrossingFile = new File(['second'], 'limit-crossing.bin', {
    type: 'application/octet-stream',
  })
  Object.defineProperty(firstFile, 'size', {
    value: 30 * 1024 * 1024,
  })
  Object.defineProperty(limitCrossingFile, 'size', {
    value: 21 * 1024 * 1024,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [firstFile, limitCrossingFile] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    error: 'limit-crossing.bin would exceed the 50 MiB attachment limit',
    attachmentNames: ['first.bin', 'limit-crossing.bin'],
    actionCalls: [],
  })
})

it('names and blocks the attachment that crosses the attachment count limit', () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const files = Array.from(
    { length: 17 },
    (_, index) => new File(['x'], `file-${String(index + 1)}.txt`),
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    error: 'file-17.txt exceeds the limit of 16 attachments per message',
    attachmentNames: [
      'file-1.txt',
      'file-2.txt',
      'file-3.txt',
      'file-4.txt',
      'file-5.txt',
      'file-6.txt',
      'file-7.txt',
      'file-8.txt',
      'file-9.txt',
      'file-10.txt',
      'file-11.txt',
      'file-12.txt',
      'file-13.txt',
      'file-14.txt',
      'file-15.txt',
      'file-16.txt',
      'file-17.txt',
    ],
    actionCalls: [],
  })
})

it('shows a server attachment validation error and retains the rejected attachment', async () => {
  const jsonResponse = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(messagesResponse([])))
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        interrupt: false,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({ error: 'file attachment data is invalid' }, 400),
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
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  const file = new File(['not an image'], 'document.pdf', {
    type: 'application/pdf',
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 10))
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentName: container.querySelector('.composer-attachment span')
      ?.textContent,
    postBody: fetcher.mock.calls[2]?.[1]?.body,
  })
  expect(snapshot()).toEqual({
    error: 'file attachment data is invalid',
    attachmentName: 'document.pdf',
    postBody: JSON.stringify({
      content: '',
      attachments: [
        {
          name: 'document.pdf',
          mimeType: 'application/pdf',
          size: 12,
          data: 'bm90IGFuIGltYWdl',
        },
      ],
    }),
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

it('reports when a draft cannot be saved after loading storage succeeds', () => {
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

it.each([
  { action: 'approve' as const, buttonIndex: 0 },
  { action: 'reject' as const, buttonIndex: 1 },
])('shows the $action error independently', async ({ action, buttonIndex }) => {
  const jsonResponse = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          {
            type: 'permission',
            tool: 'Shell',
            description: 'Run the formatter on changed files.',
          },
        ]),
      ),
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
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })
  const permissionButtons = () =>
    container.querySelectorAll<HTMLButtonElement>('.permission-actions button')
  act(() => {
    requireElement(permissionButtons().item(buttonIndex)).click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const actionRequest = fetcher.mock.calls[2]?.[0]
  const actionRequestUrl =
    typeof actionRequest === 'string'
      ? actionRequest
      : actionRequest instanceof URL
        ? actionRequest.href
        : actionRequest?.url
  const expected = JSON.stringify([
    'session pane is idle or unavailable',
    `/api/sessions/sample-session/permission/${action}`,
    [
      { label: 'Approve', disabled: false },
      { label: 'Reject', disabled: false },
    ],
  ])
  const buttonStates = Array.from(permissionButtons(), (button) => ({
    label: button.textContent,
    disabled: button.disabled,
  }))
  const actual = JSON.stringify([
    container.querySelector('[role="alert"]')?.textContent,
    actionRequestUrl,
    buttonStates,
  ])
  expect(actual).toBe(expected)
})

it.each([
  {
    action: 'approve' as const,
    pendingLabel: 'Approving…',
    sentMessage: 'Approval sent. Session is resuming…',
  },
  {
    action: 'reject' as const,
    pendingLabel: 'Rejecting…',
    sentMessage: 'Rejection sent. Session is resuming…',
  },
])(
  'keeps a successful permission $action pending until it resolves',
  async ({ action, pendingLabel, sentMessage }) => {
    const jsonResponse = (body: unknown) =>
      new Response(JSON.stringify(body), {
        headers: { 'Content-Type': 'application/json' },
      })
    let completeAction: ((response: Response) => void) | undefined
    const completePendingAction = (response: Response) => {
      const resolve = completeAction
      if (resolve === undefined) {
        throw new Error('Expected a pending action request')
      }
      resolve(response)
    }
    const unavailableActions: SessionActions = {
      ...actions,
      permission: { approve: false, reject: false },
    }
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(messagesResponse([permission])))
      .mockResolvedValueOnce(jsonResponse(actions))
      .mockImplementationOnce(
        () =>
          new Promise<Response>((resolve) => {
            completeAction = resolve
          }),
      )
      .mockResolvedValueOnce(jsonResponse(messagesResponse([permission])))
      .mockResolvedValueOnce(jsonResponse(actions))
      .mockResolvedValueOnce(jsonResponse(messagesResponse([permission])))
      .mockResolvedValueOnce(jsonResponse(unavailableActions))
      .mockResolvedValueOnce(jsonResponse(messagesResponse([])))
      .mockResolvedValueOnce(jsonResponse(unavailableActions))
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
      for (let index = 0; index < 8; index += 1) await Promise.resolve()
    })

    act(() => {
      requireElement(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        )[action === 'approve' ? 0 : 1] ?? null,
      ).click()
    })
    const duringRequest = {
      labels: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.textContent),
      disabled: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.disabled),
    }

    completePendingAction(new Response(null, { status: 204 }))
    await act(async () => {
      for (let index = 0; index < 12; index += 1) await Promise.resolve()
    })
    const afterSuccess = {
      labels: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.textContent),
      disabled: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.disabled),
      status: container.querySelector('[role="status"]')?.textContent ?? null,
    }
    const actionRequest = fetcher.mock.calls[2]?.[0]
    const actionRequestUrl =
      typeof actionRequest === 'string'
        ? actionRequest
        : actionRequest instanceof URL
          ? actionRequest.href
          : actionRequest?.url

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
    const afterCapabilityDisappears = {
      permissionCard: container.querySelector('.chat-permission-card') !== null,
      permissionButtons: container.querySelectorAll(
        '.permission-actions button',
      ).length,
      status: container.querySelector('[role="status"]')?.textContent ?? null,
    }

    act(() => {
      root?.render(
        <ConversationPane
          session={session}
          refreshKey={2}
          onSelectSession={() => undefined}
        />,
      )
    })
    await act(async () => {
      for (let index = 0; index < 12; index += 1) await Promise.resolve()
    })

    const snapshot = () => ({
      duringRequest,
      afterSuccess,
      actionRequestUrl,
      afterCapabilityDisappears,
      afterCardDisappears: {
        permissionCard: container.querySelector('.chat-permission-card'),
        status: container.querySelector('[role="status"]')?.textContent ?? null,
      },
    })
    expect(snapshot()).toEqual({
      duringRequest: {
        labels:
          action === 'approve'
            ? [pendingLabel, 'Reject']
            : ['Approve', pendingLabel],
        disabled: [true, true],
      },
      afterSuccess: {
        labels: ['Approve', 'Reject'],
        disabled: [true, true],
        status: sentMessage,
      },
      actionRequestUrl: `/api/sessions/sample-session/permission/${action}`,
      afterCapabilityDisappears: {
        permissionCard: true,
        permissionButtons: 0,
        status: null,
      },
      afterCardDisappears: { permissionCard: null, status: null },
    })
  },
)

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
      jsonResponse(
        messagesResponse(
          [{ type: 'assistant', text: 'Before update' }],
          [
            {
              id: 'fixture-task-one',
              description: 'Watch the build pipeline',
              elapsedSeconds: 18,
            },
          ],
        ),
      ),
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
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'After update' }]),
      ),
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

  const snapshot = () => ({
    transcript: container.querySelector('.chat-assistant-message')?.textContent,
    backgroundTasks: container.querySelector(
      '.conversation-background-tasks summary',
    )?.textContent,
    requestCount: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    transcript: 'After update',
    backgroundTasks: undefined,
    requestCount: 4,
  })
})

it('keeps the conversation and focused draft while a feed refresh is pending', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          { type: 'assistant', text: 'Current conversation' },
          question,
          {
            type: 'permission',
            tool: 'Shell',
            description: 'Run the project formatter.',
          },
        ]),
      ),
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
      jsonResponse(
        messagesResponse([
          question,
          {
            type: 'permission',
            tool: 'Shell',
            description: 'Run the project formatter.',
          },
        ]),
      ),
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
