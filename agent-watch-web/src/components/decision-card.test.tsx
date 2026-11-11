import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { type Decision, DecisionCard } from '#components/decision-card'

let container: HTMLDivElement
let root: Root | null = null

const decision: Decision = {
  type: 'decision',
  seq: 1,
  title: 'Use a fixed retry limit',
  why: 'The current service has no reliable timing data.',
  considered: ['Retry using the latest duration', 'Retry once per package'],
  easyToChange: 'One configuration value in the CI workflow.',
  review: null,
  createdAt: '2026-11-11T03:25:59Z',
}

function renderCard(
  overrides: Partial<Parameters<typeof DecisionCard>[0]> = {},
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <DecisionCard
        decision={decision}
        variant="conversation"
        onOk={() => Promise.resolve(true)}
        onRequestChange={() => Promise.resolve(true)}
        {...overrides}
      />,
    )
  })
}

function button(label: string) {
  const found = Array.from(container.querySelectorAll('button')).find(
    (element) => element.textContent.trim() === label,
  )
  if (found === undefined) throw new Error(`Missing button: ${label}`)
  return found
}

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) Reflect.apply(valueSetter, textarea, [value])
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
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

it('expands the conversation row when its summary is selected', () => {
  renderCard()

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.decision-card-summary'),
    ).click()
  })

  expect(container.querySelector('.decision-card-details')?.textContent).toBe(
    'Why · The current service has no reliable timing data.Also considered · Retry using the latest duration / Retry once per packageEasy to change · One configuration value in the CI workflow.OKChange…Not blocking · keeps working',
  )
})

it('sends a change comment from the expanded card', async () => {
  const onRequestChange = vi.fn(() => Promise.resolve(true))
  renderCard({ expanded: true, onRequestChange })

  act(() => {
    button('Change…').click()
  })
  const textarea = container.querySelector('textarea')
  if (textarea === null) throw new Error('Missing change request field')
  setTextareaValue(textarea, 'Add a timing sample first')
  await act(() => {
    button('Send change request').click()
    return Promise.resolve()
  })

  expect(onRequestChange.mock.calls).toEqual([['Add a timing sample first']])
})

it('sends the OK action from the expanded card', () => {
  const onOk = vi.fn(() => Promise.resolve(true))
  renderCard({ expanded: true, onOk })

  act(() => {
    button('OK').click()
  })

  expect(onOk.mock.calls).toEqual([[]])
})

it.each([
  {
    name: 'minutes',
    createdAt: '2026-11-11T06:55:00Z',
    expected: '◇ Decided · 5m ago',
  },
  {
    name: 'hours',
    createdAt: '2026-11-11T05:00:00Z',
    expected: '◇ Decided · 2h ago',
  },
  {
    name: 'days',
    createdAt: '2026-11-08T07:00:00Z',
    expected: '◇ Decided · 3d ago',
  },
])(
  'shows the decision creation time in minutes, hours, or days: $name',
  ({ createdAt, expected }) => {
    renderCard({
      decision: { ...decision, createdAt },
      variant: 'panel',
      expanded: true,
      relativeTimeReference: Date.parse('2026-11-11T07:00:00Z'),
    })

    expect(
      container.querySelector('.decision-panel-card-kicker')?.textContent,
    ).toBe(expected)
  },
)
