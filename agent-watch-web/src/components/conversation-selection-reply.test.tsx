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
}

let container: HTMLDivElement
let root: Root | null = null

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function render(messages: readonly ChatItem[]) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={session}
        messages={messages}
        actions={actions}
        loading={false}
        error={null}
        onAction={() => Promise.resolve(true)}
        onSelectSession={() => undefined}
      />,
    )
  })
}

function selectText(element: Element) {
  const textNode = element.firstChild
  if (textNode === null || textNode.nodeType !== Node.TEXT_NODE) {
    throw new Error('Expected a text node')
  }
  const range = document.createRange()
  range.setStart(textNode, 0)
  range.setEnd(textNode, textNode.textContent?.length ?? 0)
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

it('quotes each selected line and focuses the composer below it', () => {
  window.localStorage.setItem(
    `agent-watch-web:composer-draft:${session.id}`,
    'Existing note',
  )
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
  const expectedDraft = 'Existing note\n\n> First line\n> Second line\n'
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
