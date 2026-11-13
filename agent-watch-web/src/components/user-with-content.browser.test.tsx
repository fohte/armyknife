import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { formatMessageTime } from '#components/message-time'
import { UserWithContent } from '#components/user-with-content'
import type { ChatItem, RewindMode } from '#types'

const imageSource = 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs='
const now = new Date(2026, 5, 2, 0, 3)

let container: HTMLDivElement
let root: Root | null = null

function render(
  message: Extract<
    ChatItem,
    { type: 'user' | 'queued_user' | 'user_with_content' }
  >,
  props: Partial<ComponentProps<typeof UserWithContent>> = {},
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(<UserWithContent message={message} now={now} {...props} />)
  })
}

function snapshot() {
  const group = container.querySelector('.chat-user-group')
  return group === null
    ? null
    : Array.from(group.children, (child) => ({
        className: child.className,
        text: child.textContent,
        source:
          child instanceof HTMLImageElement ? child.getAttribute('src') : null,
      }))
}

function imageAttributes() {
  const image = container.querySelector('img')
  return {
    loading: image?.getAttribute('loading'),
    decoding: image?.getAttribute('decoding'),
  }
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
  vi.unstubAllGlobals()
})

it('renders the sender, text, and image in message order', () => {
  render({
    type: 'user_with_content',
    from: 'Example session',
    content: [
      { type: 'text', text: 'Before the image ' },
      { type: 'image', source: imageSource, available: true },
      { type: 'text', text: ' after it.' },
    ],
  })

  expect(snapshot()).toEqual([
    { className: 'chat-from', text: 'From Example session', source: null },
    {
      className: 'markdown-message chat-user-message',
      text: 'Before the image',
      source: null,
    },
    { className: 'chat-user-image', text: '', source: imageSource },
    {
      className: 'markdown-message chat-user-message',
      text: 'after it.',
      source: null,
    },
  ])
})

it('labels a queued prompt as You and keeps the user bubble style', () => {
  const timestamp = '2026-06-01T12:00:00Z'
  const localTime = formatMessageTime(timestamp, now) ?? 'invalid timestamp'
  render({
    type: 'queued_user',
    content: [{ type: 'text', text: 'Please rerun the example check.' }],
    timestamp,
  })

  expect(snapshot()).toEqual([
    {
      className: 'chat-from',
      text: `You · Queued · ${localTime}`,
      source: null,
    },
    {
      className: 'markdown-message chat-user-message',
      text: 'Please rerun the example check.',
      source: null,
    },
  ])
})

it('shows the local send time above a user message', () => {
  const timestamp = '2026-06-01T12:00:00Z'
  const localTime = formatMessageTime(timestamp, now) ?? 'invalid timestamp'
  render({
    type: 'user',
    text: 'Please rerun the example check.',
    timestamp,
  })

  expect(snapshot()).toEqual([
    {
      className: 'chat-from',
      text: `You · ${localTime}`,
      source: null,
    },
    {
      className: 'markdown-message chat-user-message',
      text: 'Please rerun the example check.',
      source: null,
    },
  ])
})

it('explains when an image fails to load', () => {
  render({
    type: 'user_with_content',
    content: [{ type: 'image', source: imageSource, available: true }],
  })

  act(() => {
    container.querySelector('img')?.dispatchEvent(new Event('error'))
  })

  expect(snapshot()).toEqual([
    { className: 'chat-from', text: 'You', source: null },
    {
      className: 'chat-image-unavailable',
      text: '画像はもうありません',
      source: null,
    },
  ])
})

it('loads user images lazily and decodes them asynchronously', () => {
  render({
    type: 'user_with_content',
    content: [{ type: 'image', source: imageSource, available: true }],
  })

  expect(imageAttributes()).toEqual({ loading: 'lazy', decoding: 'async' })
})

it('explains when an image is already unavailable', () => {
  render({
    type: 'user_with_content',
    content: [{ type: 'image', source: imageSource, available: false }],
  })

  expect(snapshot()).toEqual([
    { className: 'chat-from', text: 'You', source: null },
    {
      className: 'chat-image-unavailable',
      text: '画像はもうありません',
      source: null,
    },
  ])
})

it('renders a non-image attachment by its filename in message order', () => {
  render({
    type: 'user_with_content',
    content: [
      { type: 'text', text: 'Inspect this document.' },
      { type: 'file', name: 'sample-layout.html' },
    ],
  })

  expect(snapshot()).toEqual([
    { className: 'chat-from', text: 'You', source: null },
    {
      className: 'markdown-message chat-user-message',
      text: 'Inspect this document.',
      source: null,
    },
    {
      className: 'chat-user-file',
      text: 'sample-layout.html',
      source: null,
    },
  ])
})

it('renders a text-only user message', () => {
  render({ type: 'user', text: 'Please rerun the example check.' })

  expect(snapshot()).toEqual([
    { className: 'chat-from', text: 'You', source: null },
    {
      className: 'markdown-message chat-user-message',
      text: 'Please rerun the example check.',
      source: null,
    },
  ])
})

it('offers Claude rewind modes and passes the selected message text', async () => {
  const rewindCalls: Array<[string, RewindMode, string]> = []
  const onRewind = vi.fn((...args: [string, RewindMode, string]) => {
    rewindCalls.push(args)
    return Promise.resolve(true)
  })
  render(
    {
      type: 'user',
      text: 'Review this request again.',
      rewindId: 'fixture-user-message',
    },
    {
      canRewind: true,
      sessionEngine: 'claude',
      rewindMenuOpen: true,
      onRewind,
    },
  )

  const options = Array.from(
    container.querySelectorAll<HTMLButtonElement>('.chat-rewind-menu button'),
    (button) => button.textContent,
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.chat-rewind-menu button')
      ?.click()
  })
  await act(async () => {
    await Promise.resolve()
  })

  const rewindInteraction = () => ({ options, calls: rewindCalls })
  expect(rewindInteraction()).toEqual({
    options: ['Conversation and code', 'Conversation only', 'Code only'],
    calls: [
      [
        'fixture-user-message',
        'conversation_and_code',
        'Review this request again.',
      ],
    ],
  })
})

it('offers only conversation rewind for Codex messages', () => {
  render(
    {
      type: 'user',
      text: 'Review this request again.',
      rewindId: 'fixture-user-message',
    },
    {
      canRewind: true,
      sessionEngine: 'codex',
      rewindMenuOpen: true,
    },
  )

  expect(
    Array.from(
      container.querySelectorAll<HTMLButtonElement>('.chat-rewind-menu button'),
      (button) => button.textContent,
    ),
  ).toEqual(['Conversation only'])
})
