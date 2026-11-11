import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { PeerMessage } from '#components/peer-message'
import type { ChatItem } from '#types'

let container: HTMLDivElement
let root: Root | null = null

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

it.each([
  {
    sessionId: 'fixture-untracked-session',
    sessionTitle: undefined,
    direction: 'to',
    label: 'TOfixture-untracked-session (ended)',
  },
  {
    sessionId: 'fixture-parent-session',
    sessionTitle: 'Fixture parent',
    direction: 'from',
    label: 'FROMFixture parent (ended)',
  },
  {
    sessionId: 'unknown',
    sessionTitle: undefined,
    direction: 'from',
    label: 'FROMarmyknife',
  },
] as const)(
  'shows $label without a session card',
  ({ sessionId, sessionTitle, direction, label }) => {
    root = createRoot(container)
    const onSelectSession = vi.fn<(id: string) => void>()
    const message = {
      type: 'peer',
      direction,
      session_id: sessionId,
      ...(sessionTitle === undefined ? {} : { session_title: sessionTitle }),
      content: [{ type: 'text', text: 'A fictional peer message.' }],
    } satisfies Extract<ChatItem, { type: 'peer' }>

    act(() => {
      root?.render(
        <PeerMessage
          message={message}
          onSelectSession={onSelectSession}
          sessions={[]}
        />,
      )
    })

    act(() => {
      container
        .querySelector('.chat-peer-content')
        ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const getActual = () => ({
      label: container.querySelector('.chat-peer-label')?.textContent ?? null,
      linkCount: container.querySelectorAll('.session-link-trigger').length,
      card: document.querySelector('.session-link-card')?.textContent ?? null,
      contentRole:
        container.querySelector('.chat-peer-content')?.getAttribute('role') ??
        null,
      contentClass: container.querySelector('.chat-peer-content')?.className,
      selectedIds: onSelectSession.mock.calls.map(([id]) => id),
    })
    expect(getActual()).toEqual({
      label,
      linkCount: 0,
      card: null,
      contentRole: null,
      contentClass: 'chat-peer-content chat-peer-content-unavailable',
      selectedIds: [],
    })
  },
)

it('keeps Markdown block spacing inside peer messages', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <PeerMessage
        message={{
          type: 'peer',
          direction: 'from',
          session_id: 'unknown',
          content: [
            {
              type: 'text',
              text: `<delegation-update>\nA summary.\n\n- First item\n- Second item\n</delegation-update>`,
            },
          ],
        }}
        onSelectSession={() => undefined}
        sessions={[]}
      />,
    )
  })

  const markdown = container.querySelector('.chat-peer-text')
  if (!(markdown instanceof HTMLElement)) {
    throw new Error('Expected peer Markdown content to render')
  }
  const blocks = Array.from(markdown.children)
  const paragraph = markdown.querySelector('p')
  if (paragraph === null) throw new Error('Expected a Markdown paragraph')
  const blockGaps = blocks.slice(1).map((block, index) => {
    const previous = blocks[index]
    if (previous === undefined) throw new Error('Expected a preceding block')
    return Math.round(
      block.getBoundingClientRect().top -
        previous.getBoundingClientRect().bottom,
    )
  })

  const getActual = () => ({
    blockGaps,
    blockTags: blocks.map((block) => block.tagName),
    markdownWhiteSpace: getComputedStyle(markdown).whiteSpace,
    paragraphWhiteSpace: getComputedStyle(paragraph).whiteSpace,
  })
  expect(getActual()).toEqual({
    blockGaps: [12, 12],
    blockTags: ['H2', 'P', 'UL'],
    markdownWhiteSpace: 'normal',
    paragraphWhiteSpace: 'pre-line',
  })
})
