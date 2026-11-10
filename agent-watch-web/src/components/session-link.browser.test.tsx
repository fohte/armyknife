import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { SessionLink } from '#components/session-link'
import type { Session } from '#types'

const linkedSession: Session = {
  id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  category: 'in_progress',
  isPaused: false,
  kind: 'Plan',
  title: 'Inspect an example workflow',
  preview: null,
  elapsedSeconds: 30,
  unread: false,
  task: {
    id: 'sample-task',
    number: 24,
    title: 'Inspect the example workflow',
    priority: 2,
  },
  repo: 'sample-org/example-service',
  project: 'Example service',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

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

it('shows session details on hover', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink session={linkedSession}>sample-session</SessionLink>,
    )
  })

  const link = container.querySelector('a')
  if (link === null) throw new Error('Expected a session link')
  await act(async () => {
    await userEvent.hover(link)
    await new Promise((resolve) => setTimeout(resolve, 250))
  })

  const card = document.querySelector('.session-link-card')
  const getActual = () => ({
    linkClass: link.className,
    linkHref: link.getAttribute('href'),
    cardText: card?.textContent ?? null,
  })
  expect(getActual()).toEqual({
    linkClass: 'session-link-trigger',
    linkHref: '?session=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
    cardText:
      'In progresstq #24 Inspect the example workflowrepo sample-org/example-service',
  })
})

it('keeps the card open while the pointer is over it', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink session={linkedSession}>sample-session</SessionLink>,
    )
  })

  const link = container.querySelector('a')
  if (link === null) throw new Error('Expected a session link')
  await act(async () => {
    await userEvent.hover(link)
    await new Promise((resolve) => setTimeout(resolve, 250))
  })

  const card = document.querySelector('.session-link-card')
  if (card === null) throw new Error('Expected the session detail card')
  await act(async () => {
    await userEvent.hover(card)
    await new Promise((resolve) => setTimeout(resolve, 50))
  })

  expect(document.querySelector('.session-link-card')?.textContent).toBe(
    'In progresstq #24 Inspect the example workflowrepo sample-org/example-service',
  )
})

it('labels a paused session in the detail card', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink open session={{ ...linkedSession, isPaused: true }}>
        sample-session
      </SessionLink>,
    )
  })

  expect(document.querySelector('.session-link-card')?.textContent).toBe(
    'In progress (paused)tq #24 Inspect the example workflowrepo sample-org/example-service',
  )
})

it('explains when no tq task is linked', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink open session={{ ...linkedSession, task: null }}>
        sample-session
      </SessionLink>,
    )
  })

  expect(document.querySelector('.session-link-card')?.textContent).toBe(
    'In progressNo linked tq taskrepo sample-org/example-service',
  )
})

it('opens the session when clicked while the card is visible', async () => {
  const onSelectSession = vi.fn<(id: string) => void>()
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink
        onSelectSession={onSelectSession}
        open
        session={linkedSession}
      >
        sample-session
      </SessionLink>,
    )
  })

  const link = container.querySelector('a')
  if (link === null) throw new Error('Expected a session link')
  await userEvent.click(link)

  const getActual = () => ({
    linkHref: link.getAttribute('href'),
    cardText: document.querySelector('.session-link-card')?.textContent ?? null,
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  expect(getActual()).toEqual({
    linkHref: '?session=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
    cardText:
      'In progresstq #24 Inspect the example workflowrepo sample-org/example-service',
    selectedIds: [linkedSession.id],
  })
})

it('shows the card when a keyboard user focuses the link', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink session={linkedSession}>sample-session</SessionLink>,
    )
  })

  const link = container.querySelector('a')
  if (link === null) throw new Error('Expected a session link')
  await act(async () => userEvent.tab())

  expect(document.querySelector('.session-link-card')?.textContent).toBe(
    'In progresstq #24 Inspect the example workflowrepo sample-org/example-service',
  )
})
