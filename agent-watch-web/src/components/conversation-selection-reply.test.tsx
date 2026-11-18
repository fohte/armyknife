import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationPaneView } from '#components/conversation-pane-view'
import type { ChatItem, Session, SessionActions } from '#types'

const session: Session = {
  id: 'quote-reply-session',
  engine: 'claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Message',
  title: 'Review the change',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'service',
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

let container: HTMLDivElement
let root: Root | null = null

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function render(messages: readonly ChatItem[], isMobile = false) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={session}
        messages={messages}
        isMobile={isMobile}
        actions={actions}
        loading={false}
        error={null}
        onAction={() => Promise.resolve(true)}
        onSelectSession={() => undefined}
      />,
    )
  })
}

function selectText(element: Element, rects?: readonly DOMRect[]) {
  const textNode = element.firstChild
  if (textNode === null || textNode.nodeType !== Node.TEXT_NODE) {
    throw new Error('Expected a text node')
  }
  const range = document.createRange()
  range.setStart(textNode, 0)
  range.setEnd(textNode, textNode.textContent?.length ?? 0)
  if (rects !== undefined) {
    Object.defineProperty(range, 'getClientRects', { value: () => rects })
  }
  act(() => {
    const selection = window.getSelection()
    selection?.removeAllRanges()
    selection?.addRange(range)
    document.dispatchEvent(new Event('selectionchange'))
  })
}

function selectRange(
  start: Node,
  startOffset: number,
  end: Node,
  endOffset: number,
) {
  const range = document.createRange()
  range.setStart(start, startOffset)
  range.setEnd(end, endOffset)
  act(() => {
    const selection = window.getSelection()
    selection?.removeAllRanges()
    selection?.addRange(range)
    document.dispatchEvent(new Event('selectionchange'))
  })
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
  window.getSelection()?.removeAllRanges()
  container.remove()
  window.localStorage.clear()
  vi.unstubAllGlobals()
})

it.each([
  {
    name: 'existing draft',
    initialDraft: 'Existing note',
    expectedDraft: 'Existing note\n\n> First line\n> Second line\n\n',
  },
  {
    name: 'empty draft',
    initialDraft: '',
    expectedDraft: '> First line\n> Second line\n\n',
  },
])(
  'quotes each selected line into an $name',
  ({ initialDraft, expectedDraft }) => {
    if (initialDraft !== '') {
      window.localStorage.setItem(
        `agent-watch-web:composer-draft:${session.id}`,
        initialDraft,
      )
    }
    render([{ type: 'assistant', text: 'First line\nSecond line' }])

    const paragraph = requireElement(
      container.querySelector('.chat-assistant-message p'),
    )
    selectText(paragraph)
    const replyButton = requireElement(
      container.querySelector<HTMLButtonElement>(
        '.conversation-selection-reply button',
      ),
    )
    act(() => {
      replyButton.click()
    })

    const textarea = requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    )
    const snapshot = () => ({
      draft: textarea.value,
      focused: document.activeElement === textarea,
      selectionStart: textarea.selectionStart,
      selectionEnd: textarea.selectionEnd,
      menuVisible:
        container.querySelector('.conversation-selection-reply') !== null,
    })
    expect(snapshot()).toEqual({
      draft: expectedDraft,
      focused: true,
      selectionStart: expectedDraft.length,
      selectionEnd: expectedDraft.length,
      menuVisible: false,
    })
  },
)

it.each([
  {
    kind: 'paragraph',
    text: 'Earlier paragraph\n\nFinal paragraph',
    selector: '.chat-assistant-message p:last-child',
    expectedDraft: '> Final paragraph\n\n',
  },
  {
    kind: 'list item',
    text: 'Earlier paragraph\n\n- Earlier item\n- Final item',
    selector: '.chat-assistant-message li:last-child',
    expectedDraft: '> Final item\n\n',
  },
])(
  'quotes the last $kind when selection ends at the following message',
  ({ text, selector, expectedDraft }) => {
    render([
      { type: 'assistant', text },
      { type: 'assistant', text: 'Following message' },
    ])

    const lastLine = requireElement(container.querySelector(selector))
    const lastLineText = lastLine.firstChild
    if (lastLineText === null || lastLineText.nodeType !== Node.TEXT_NODE) {
      throw new Error('Expected the last line to contain a text node')
    }
    const messageParagraphs = container.querySelectorAll(
      '.chat-assistant-message p',
    )
    const nextMessageText = requireElement(
      messageParagraphs.item(messageParagraphs.length - 1),
    ).firstChild
    if (
      nextMessageText === null ||
      nextMessageText.nodeType !== Node.TEXT_NODE
    ) {
      throw new Error('Expected the following message to contain a text node')
    }
    selectRange(lastLineText, 0, nextMessageText, 0)

    act(() => {
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.conversation-selection-reply button',
        ),
      ).click()
    })

    expect(
      requireElement(container.querySelector<HTMLTextAreaElement>('textarea'))
        .value,
    ).toBe(expectedDraft)
  },
)

it('does not offer replies when selected text crosses assistant messages', () => {
  render([
    { type: 'assistant', text: 'First message' },
    { type: 'assistant', text: 'Second message' },
  ])
  const messageTexts = container.querySelectorAll('.chat-assistant-message p')
  const firstText = requireElement(messageTexts.item(0)).firstChild
  const secondText = requireElement(messageTexts.item(1)).firstChild
  if (
    firstText === null ||
    firstText.nodeType !== Node.TEXT_NODE ||
    secondText === null ||
    secondText.nodeType !== Node.TEXT_NODE
  ) {
    throw new Error('Expected assistant messages to contain text nodes')
  }
  selectRange(firstText, 0, secondText, 1)

  expect(
    container.querySelector('.conversation-selection-reply') !== null,
  ).toBe(false)
})

it('does not offer replies for text outside an assistant message', () => {
  render([{ type: 'user', text: 'A user message' }])
  selectText(requireElement(container.querySelector('.chat-user-message p')))

  const snapshot = () => ({
    menuVisible:
      container.querySelector('.conversation-selection-reply') !== null,
  })
  expect(snapshot()).toEqual({ menuVisible: false })
})

it('keeps the mobile reply menu visible near the bottom edge', () => {
  render([{ type: 'assistant', text: 'Select this sentence.' }], true)
  const selectionTop = window.innerHeight - 24
  selectText(
    requireElement(container.querySelector('.chat-assistant-message p')),
    [new DOMRect(20, selectionTop, 100, 16)],
  )

  const menu = requireElement(
    container.querySelector<HTMLElement>('.conversation-selection-reply'),
  )
  const menuTop = Number.parseFloat(menu.style.top)
  const maxTop = Math.max(8, window.innerHeight - 50)
  const expectedTop = Math.min(Math.max(selectionTop - 50, 8), maxTop)
  const snapshot = () => ({
    top: menuTop,
    aboveSelection: menuTop < selectionTop,
    insideViewport: menuTop >= 8 && menuTop + 42 <= window.innerHeight - 8,
  })
  expect(snapshot()).toEqual({
    top: expectedTop,
    aboveSelection: true,
    insideViewport: true,
  })
})

it.each([
  { dismissal: 'Escape', event: 'escape' },
  { dismissal: 'conversation scroll', event: 'scroll' },
])('$dismissal closes the reply menu', ({ event }) => {
  render([{ type: 'assistant', text: 'Select this sentence.' }])
  selectText(
    requireElement(container.querySelector('.chat-assistant-message p')),
  )
  const conversationScroll = requireElement(
    container.querySelector('.conversation-scroll'),
  )

  act(() => {
    if (event === 'escape') {
      document.dispatchEvent(
        new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }),
      )
    } else {
      conversationScroll.dispatchEvent(new Event('scroll', { bubbles: true }))
    }
  })

  const snapshot = () => ({
    menuVisible:
      container.querySelector('.conversation-selection-reply') !== null,
  })
  expect(snapshot()).toEqual({ menuVisible: false })
})
