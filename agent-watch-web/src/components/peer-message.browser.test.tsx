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
    label: 'TOfixture-untracked-session',
  },
  { sessionId: 'unknown', label: 'FROMUnknown session' },
])('shows $label without a session card', ({ sessionId, label }) => {
  root = createRoot(container)
  const message = {
    type: 'peer',
    direction: sessionId === 'unknown' ? 'from' : 'to',
    session_id: sessionId,
    content: [{ type: 'text', text: 'A fictional peer message.' }],
  } satisfies Extract<ChatItem, { type: 'peer' }>

  act(() => {
    root?.render(
      <PeerMessage
        message={message}
        onSelectSession={() => undefined}
        sessions={[]}
      />,
    )
  })

  const getActual = () => ({
    label: container.querySelector('.chat-peer-label')?.textContent ?? null,
    linkCount: container.querySelectorAll('.session-link-trigger').length,
    card: document.querySelector('.session-link-card')?.textContent ?? null,
    contentRole:
      container.querySelector('.chat-peer-content')?.getAttribute('role') ??
      null,
  })
  expect(getActual()).toEqual({
    label,
    linkCount: 0,
    card: null,
    contentRole: null,
  })
})
