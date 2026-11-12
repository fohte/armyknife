import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'sample-session',
  category: 'in_progress',
  isPaused: false,
  kind: null,
  title: 'Finish the retry policy update',
  preview: null,
  elapsedSeconds: 120,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'service reliability',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const decisions: Decision[] = [
  {
    type: 'decision',
    seq: 1,
    title: 'Use a fixed retry limit',
    why: 'The current service has no reliable timing data.',
    considered: ['Retry using the latest duration'],
    easyToChange: 'One configuration value in the CI workflow.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  },
  {
    type: 'decision',
    seq: 2,
    title: 'Keep the current backoff interval',
    why: 'It already matches the service limits.',
    considered: ['Use a shorter interval'],
    easyToChange: 'The interval is one configuration value.',
    review: null,
    createdAt: '2026-11-11T03:27:59Z',
  },
]

function renderPanel(onOk = vi.fn(() => Promise.resolve(true))) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={decisions}
        session={session}
        selectedSeq={1}
        onSelect={vi.fn()}
        onOk={onOk}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })
  return onOk
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
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('uses Y to approve the selected decision and select the next pending one', async () => {
  const onOk = vi.fn(() => Promise.resolve(true))
  const onSelect = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={decisions}
        session={session}
        selectedSeq={1}
        onSelect={onSelect}
        onOk={onOk}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })
  const headerButton = document.createElement('button')
  document.body.append(headerButton)
  headerButton.focus()

  await act(() => {
    headerButton.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'y', bubbles: true }),
    )
    return Promise.resolve()
  })

  const snapshot = () => ({ ok: onOk.mock.calls, next: onSelect.mock.calls })
  expect(snapshot()).toEqual({
    ok: [[1]],
    next: [[2]],
  })
  headerButton.remove()
})

it('hides ok decisions and keeps change requests visible', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={decisions.map((decision, index) =>
          index === 0
            ? { ...decision, review: 'ok' }
            : { ...decision, review: 'change' },
        )}
        session={session}
        selectedSeq={2}
        onSelect={vi.fn()}
        onOk={vi.fn(() => Promise.resolve(true))}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })

  const snapshot = Array.from(
    container.querySelectorAll('.decision-card--panel'),
    (card) => card.getAttribute('data-review'),
  )
  const visibleState = () => ({
    cards: snapshot,
    reviewProgress: container.querySelector('.decisions-panel-header span')
      ?.textContent,
    changeFeedback: container.querySelector('.decision-card-review-state')
      ?.textContent,
  })
  expect(visibleState()).toEqual({
    cards: ['change'],
    reviewProgress: '2 / 2 reviewed',
    changeFeedback: 'Change requested',
  })
})

it('confirms when all decisions are approved', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={decisions.map((decision) => ({
          ...decision,
          review: 'ok',
        }))}
        session={session}
        selectedSeq={1}
        onSelect={vi.fn()}
        onOk={vi.fn(() => Promise.resolve(true))}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })

  const snapshot = () => ({
    progress: container.querySelector('.decisions-panel-header span')
      ?.textContent,
    cards: Array.from(
      container.querySelectorAll('.decision-card--panel'),
      (card) => card.getAttribute('data-review'),
    ),
    emptyState: container.querySelector('.decisions-panel-empty')?.textContent,
  })
  expect(snapshot()).toEqual({
    progress: '2 / 2 reviewed',
    cards: [],
    emptyState: 'All decisions are approved.',
  })
})

it('shows unreviewed decisions before change requests', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={decisions.map((decision, index) =>
          index === 0 ? { ...decision, review: 'change' } : decision,
        )}
        session={session}
        selectedSeq={2}
        onSelect={vi.fn()}
        onOk={vi.fn(() => Promise.resolve(true))}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })

  const snapshot = () => ({
    reviews: Array.from(
      container.querySelectorAll('.decision-card--panel'),
      (card) => card.getAttribute('data-review'),
    ),
    progress: container.querySelector('.decisions-panel-header span')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    reviews: ['unreviewed', 'change'],
    progress: '1 / 2 reviewed',
  })
})

it('keeps the selected decision when the Y action fails', async () => {
  const onOk = vi.fn(() => Promise.resolve(false))
  const onSelect = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={decisions}
        session={session}
        selectedSeq={1}
        onSelect={onSelect}
        onOk={onOk}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })

  await act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'y' }))
    return Promise.resolve()
  })

  const snapshot = () => ({
    ok: onOk.mock.calls,
    selection: onSelect.mock.calls,
  })
  expect(snapshot()).toEqual({ ok: [[1]], selection: [] })
})

it('skips reviewed decisions when Y selects the next pending decision', async () => {
  const firstDecision = decisions.find(({ seq }) => seq === 1)
  const secondDecision = decisions.find(({ seq }) => seq === 2)
  if (firstDecision === undefined || secondDecision === undefined) {
    throw new Error('Expected the test decisions to be present')
  }
  const reviewedDecision: Decision = {
    ...secondDecision,
    review: 'ok',
  }
  const lastDecision: Decision = {
    ...secondDecision,
    seq: 3,
  }
  const onOk = vi.fn(() => Promise.resolve(true))
  const onSelect = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <DecisionsPanel
        decisions={[firstDecision, reviewedDecision, lastDecision]}
        session={session}
        selectedSeq={1}
        onSelect={onSelect}
        onOk={onOk}
        onChange={vi.fn(() => Promise.resolve(true))}
      />,
    )
  })

  await act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'y' }))
    return Promise.resolve()
  })

  const snapshot = () => ({
    ok: onOk.mock.calls,
    selection: onSelect.mock.calls,
  })
  expect(snapshot()).toEqual({ ok: [[1]], selection: [[3]] })
})

it('does not use Y while a text field is focused', () => {
  const onOk = renderPanel()
  const textarea = document.createElement('textarea')
  document.body.append(textarea)

  act(() => {
    textarea.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'y', bubbles: true }),
    )
  })

  expect(onOk.mock.calls).toEqual([])
  textarea.remove()
})

it('uses C to open and focus the change comment field', () => {
  renderPanel()
  const cardButton = document.createElement('button')
  document.body.append(cardButton)
  cardButton.focus()

  act(() => {
    cardButton.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'c', bubbles: true }),
    )
  })

  const snapshot = () => ({
    field: container.querySelector<HTMLTextAreaElement>('textarea')?.id,
    focused: document.activeElement?.id,
  })
  expect(snapshot()).toEqual({
    field: 'decision-change-1',
    focused: 'decision-change-1',
  })
  cardButton.remove()
})

it('uses Enter to jump to the selected conversation row', () => {
  renderPanel()
  const row = document.createElement('div')
  row.id = 'decision-row-1'
  const scrollIntoView = vi.fn()
  row.scrollIntoView = scrollIntoView
  document.body.append(row)

  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }))
  })

  expect(scrollIntoView.mock.calls).toEqual([
    [{ behavior: 'smooth', block: 'center' }],
  ])
  row.remove()
})

it('preserves native Enter and Space activation on buttons and links', () => {
  renderPanel()
  const row = document.createElement('div')
  row.id = 'decision-row-1'
  const scrollIntoView = vi.fn()
  row.scrollIntoView = scrollIntoView
  const button = document.createElement('button')
  const link = document.createElement('a')
  link.href = '#decision-row-1'
  const roleLink = document.createElement('div')
  roleLink.setAttribute('role', 'link')
  document.body.append(row, button, link, roleLink)
  button.focus()
  const buttonEvents = ['Enter', ' '].map(
    (key) =>
      new KeyboardEvent('keydown', {
        key,
        bubbles: true,
        cancelable: true,
      }),
  )
  const linkEvent = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  })
  const roleLinkEvent = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  })

  act(() => {
    for (const event of buttonEvents) button.dispatchEvent(event)
    link.dispatchEvent(linkEvent)
    roleLink.dispatchEvent(roleLinkEvent)
  })

  const snapshot = () => ({
    prevented: [
      ...buttonEvents.map((event) => event.defaultPrevented),
      linkEvent.defaultPrevented,
      roleLinkEvent.defaultPrevented,
    ],
    scroll: scrollIntoView.mock.calls,
  })
  expect(snapshot()).toEqual({
    prevented: [false, false, false, false],
    scroll: [],
  })
  row.remove()
  button.remove()
  link.remove()
  roleLink.remove()
})
