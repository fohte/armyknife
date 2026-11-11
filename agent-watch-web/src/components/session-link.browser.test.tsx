import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { SessionLink } from '#components/session-link'
import { SessionLinkElapsedSecondsContext } from '#components/session-link-elapsed-seconds'
import type { SessionLinkTarget } from '#types'

const linkedSession: SessionLinkTarget = {
  id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
  engine: 'Claude',
  workType: 'PM',
  kind: 'Plan',
  title: 'Inspect an example workflow',
  preview: 'Review the sample configuration.',
  elapsedSeconds: 120,
  task: {
    id: 'sample-task',
    number: 24,
    title: 'Inspect the example workflow',
    priority: 2,
  },
  linkedTaskCount: 3,
  repo: 'sample-org/example-service',
}
const openBesideModifier =
  getCommandPaletteShortcut(navigator.platform).modifier === 'metaKey'
    ? '⌘'
    : 'Ctrl'
const expectedCardText =
  `Inspect an example workflowPM · ClaudePlan · Review the sample configuration. · 2m` +
  `P2 #24Inspect the example workflow +2sample-org/example-service` +
  `Click to open · ${openBesideModifier}-click to open beside`

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
    kind:
      card
        ?.querySelector('.session-link-card-kind')
        ?.getAttribute('data-session-kind') ?? null,
  })
  expect(getActual()).toEqual({
    linkClass: 'session-link-trigger',
    linkHref: '?session=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
    cardText: expectedCardText,
    kind: 'plan',
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
    expectedCardText,
  )
})

it('hides unavailable session details', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink
        open
        session={{
          ...linkedSession,
          engine: undefined,
          workType: null,
          kind: null,
          preview: null,
          elapsedSeconds: undefined,
          task: null,
          repo: '',
        }}
      >
        sample-session
      </SessionLink>,
    )
  })

  const card = document.querySelector('.session-link-card')
  const getActual = () => ({
    heading:
      card?.querySelector('.session-link-card-heading')?.textContent ?? null,
    status:
      card?.querySelector('.session-link-card-status')?.textContent ?? null,
    task:
      card?.querySelector('.session-link-card-task-row')?.textContent ?? null,
    repo: card?.querySelector('.session-link-card-repo')?.textContent ?? null,
    hint: card?.querySelector('.session-link-card-hint')?.textContent ?? null,
  })
  expect(getActual()).toEqual({
    heading: 'Inspect an example workflow',
    status: null,
    task: null,
    repo: null,
    hint: `Click to open · ${openBesideModifier}-click to open beside`,
  })
})

it('updates elapsed time without changing the session target', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLinkElapsedSecondsContext.Provider
        value={new Map([[linkedSession.id, 120]])}
      >
        <SessionLink open session={linkedSession}>
          sample-session
        </SessionLink>
      </SessionLinkElapsedSecondsContext.Provider>,
    )
  })

  act(() => {
    root?.render(
      <SessionLinkElapsedSecondsContext.Provider
        value={new Map([[linkedSession.id, 180]])}
      >
        <SessionLink open session={linkedSession}>
          sample-session
        </SessionLink>
      </SessionLinkElapsedSecondsContext.Provider>,
    )
  })

  const getActual = () => ({
    status:
      document.querySelector('.session-link-card-status')?.textContent ?? null,
    details:
      document.querySelector('.session-link-card-status-details')
        ?.textContent ?? null,
  })
  expect(getActual()).toEqual({
    status: 'Plan · Review the sample configuration. · 3m',
    details: ' · Review the sample configuration. · 3m',
  })
})

it('omits the task row when no tq task is linked', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionLink open session={{ ...linkedSession, task: null }}>
        sample-session
      </SessionLink>,
    )
  })

  const getActual = () => ({
    task:
      document.querySelector('.session-link-card-task-row')?.textContent ??
      null,
    repo:
      document.querySelector('.session-link-card-repo')?.textContent ?? null,
  })
  expect(getActual()).toEqual({
    task: null,
    repo: 'sample-org/example-service',
  })
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
    cardText: expectedCardText,
    selectedIds: [linkedSession.id],
  })
})

it.each([
  ['Meta', { metaKey: true }],
  ['Control', { ctrlKey: true }],
] as const)(
  '%s-click keeps the native session link behavior',
  (_, modifiers) => {
    root = createRoot(container)
    const onSelectSession = vi.fn<(id: string) => void>()

    act(() => {
      root?.render(
        <SessionLink onSelectSession={onSelectSession} session={linkedSession}>
          sample-session
        </SessionLink>,
      )
    })

    const link = container.querySelector('a')
    if (link === null) throw new Error('Expected a session link')
    link.removeAttribute('href')
    const click = new MouseEvent('click', {
      bubbles: true,
      cancelable: true,
      ...modifiers,
    })
    link.dispatchEvent(click)

    const getActual = () => ({
      defaultPrevented: click.defaultPrevented,
      selectedIds: onSelectSession.mock.calls.map(([id]) => id),
    })
    expect(getActual()).toEqual({
      defaultPrevented: false,
      selectedIds: [],
    })
  },
)

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
    expectedCardText,
  )
})
