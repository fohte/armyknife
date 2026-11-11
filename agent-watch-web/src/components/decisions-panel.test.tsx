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
