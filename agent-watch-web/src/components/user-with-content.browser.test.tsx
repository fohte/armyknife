import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { UserWithContent } from '#components/user-with-content'
import type { ChatItem } from '#types'

const imageSource = 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs='

let container: HTMLDivElement
let root: Root | null = null

function render(
  message: Extract<ChatItem, { type: 'user' | 'user_with_content' }>,
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(<UserWithContent message={message} />)
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

it('explains when an image fails to load', () => {
  render({
    type: 'user_with_content',
    content: [{ type: 'image', source: imageSource, available: true }],
  })

  act(() => {
    container.querySelector('img')?.dispatchEvent(new Event('error'))
  })

  expect(snapshot()).toEqual([
    {
      className: 'chat-image-unavailable',
      text: '画像はもうありません',
      source: null,
    },
  ])
})

it('explains when an image is already unavailable', () => {
  render({
    type: 'user_with_content',
    content: [{ type: 'image', source: imageSource, available: false }],
  })

  expect(snapshot()).toEqual([
    {
      className: 'chat-image-unavailable',
      text: '画像はもうありません',
      source: null,
    },
  ])
})

it('renders a text-only user message', () => {
  render({ type: 'user', text: 'Please rerun the example check.' })

  expect(snapshot()).toEqual([
    {
      className: 'markdown-message chat-user-message',
      text: 'Please rerun the example check.',
      source: null,
    },
  ])
})
