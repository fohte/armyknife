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

  await act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'y' }))
    return Promise.resolve()
  })

  const snapshot = () => ({ ok: onOk.mock.calls, next: onSelect.mock.calls })
  expect(snapshot()).toEqual({
    ok: [[1]],
    next: [[2]],
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

  act(() => {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'c' }))
  })

  const snapshot = () => ({
    field: container.querySelector<HTMLTextAreaElement>('textarea')?.id,
    focused: document.activeElement?.id,
  })
  expect(snapshot()).toEqual({
    field: 'decision-change-1',
    focused: 'decision-change-1',
  })
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

it('leaves Enter on a focused button to the browser', () => {
  renderPanel()
  const row = document.createElement('div')
  row.id = 'decision-row-1'
  const scrollIntoView = vi.fn()
  row.scrollIntoView = scrollIntoView
  const button = document.createElement('button')
  document.body.append(row, button)
  const event = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  })

  act(() => {
    button.dispatchEvent(event)
  })

  const snapshot = () => ({
    prevented: event.defaultPrevented,
    scroll: scrollIntoView.mock.calls,
  })
  expect(snapshot()).toEqual({ prevented: false, scroll: [] })
  row.remove()
  button.remove()
})
