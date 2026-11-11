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
    label: 'TOfixture-untracked-session',
  },
  {
    sessionId: 'fixture-parent-session',
    sessionTitle: 'Fixture parent',
    direction: 'from',
    label: 'FROMFixture parent (fixture-parent-session)',
  },
  {
    sessionId: 'unknown',
    sessionTitle: undefined,
    direction: 'from',
    label: 'FROMUnknown session',
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
