import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import { type Decision, DecisionCard } from '#components/decision-card'
import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import type { SessionLinkTarget } from '#types'

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

const linkedSessionId = '11111111-1111-4111-8111-111111111111'
const linkedSession: SessionLinkTarget = {
  id: linkedSessionId,
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}
const onSelectSession = vi.fn<(id: string) => void>()

function renderCard(
  overrides: Partial<Parameters<typeof DecisionCard>[0]> = {},
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <MarkdownSessionLinksProvider
        sessions={[linkedSession]}
        onSelectSession={onSelectSession}
      >
        <DecisionCard
          decision={decision}
          variant="conversation"
          onOk={() => Promise.resolve(true)}
          onRequestChange={() => Promise.resolve(true)}
          {...overrides}
        />
      </MarkdownSessionLinksProvider>,
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
  onSelectSession.mockReset()
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

it('renders decision details as markdown with linked session IDs', () => {
  renderCard({
    expanded: true,
    decision: {
      ...decision,
      why: `Follow the approach in ${linkedSessionId}.`,
      considered: [
        `Keep the behavior from ${linkedSessionId}.`,
        'Use **a separate implementation**.',
      ],
      easyToChange: `Revisit the settings in ${linkedSessionId}.`,
    },
  })

  const links = Array.from(
    container.querySelectorAll<HTMLAnchorElement>('.decision-card-details a'),
  )
  act(() => {
    links.forEach((link) => {
      link.dispatchEvent(
        new MouseEvent('click', { bubbles: true, cancelable: true }),
      )
    })
  })

  const getActual = () => ({
    markdown: Array.from(
      container.querySelectorAll('.decision-card-details .markdown-message'),
      (message) => message.textContent,
    ),
    links: links.map((link) => ({
      text: link.textContent,
      href: link.getAttribute('href'),
    })),
    selectedSessions: onSelectSession.mock.calls.map(([id]) => id),
  })
  expect(getActual()).toEqual({
    markdown: [
      `Follow the approach in ${linkedSessionId}.`,
      `Keep the behavior from ${linkedSessionId}.`,
      'Use a separate implementation.',
      `Revisit the settings in ${linkedSessionId}.`,
    ],
    links: Array.from({ length: 3 }, () => ({
      text: linkedSessionId,
      href: `?session=${linkedSessionId}`,
    })),
    selectedSessions: Array.from({ length: 3 }, () => linkedSessionId),
  })
})

it('expands the conversation row when its summary is selected', () => {
  renderCard()

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.decision-card-summary'),
    ).click()
  })

  expect(container.querySelector('.decision-card-details')?.textContent).toBe(
    'Why · The current service has no reliable timing data.Also considered · Retry using the latest durationRetry once per packageEasy to change · One configuration value in the CI workflow.OKChange…Not blocking · keeps working',
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

it('restores a change request after reload and clears it after sending', async () => {
  const sessionId = crypto.randomUUID()
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'decision',
    sessionId,
    decision.seq,
    'change-comment',
  ])}`
  const onRequestChange = vi.fn(() => Promise.resolve(true))
  const cardProps = { sessionId, expanded: true, onRequestChange }
  renderCard(cardProps)
  act(() => {
    button('Change…').click()
  })
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    'Please include the review timing.',
  )
  const storedDraftBeforeReload = window.localStorage.getItem(storageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderCard(cardProps)
  act(() => {
    button('Change…').click()
  })

  const restoredComment = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const persistedBeforeSend = restoredComment.value
  await act(async () => {
    button('Send change request').click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    restoredComment: persistedBeforeSend,
    changeCalls: onRequestChange.mock.calls,
    storedDraftBeforeReload,
    storedDraft: window.localStorage.getItem(storageKey),
  })

  expect(snapshot()).toEqual({
    restoredComment: 'Please include the review timing.',
    changeCalls: [['Please include the review timing.']],
    storedDraftBeforeReload: 'Please include the review timing.',
    storedDraft: null,
  })
})

it('keeps change requests after an unsuccessful send and reload', async () => {
  const sessionId = crypto.randomUUID()
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'decision',
    sessionId,
    decision.seq,
    'change-comment',
  ])}`
  const onRequestChange = vi.fn(() => Promise.resolve(false))
  const cardProps = { sessionId, expanded: true, onRequestChange }
  renderCard(cardProps)
  act(() => {
    button('Change…').click()
  })
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    'Keep the request available for a retry.',
  )
  await act(async () => {
    button('Send change request').click()
    await Promise.resolve()
  })
  const storedDraftAfterFailure = window.localStorage.getItem(storageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderCard(cardProps)
  act(() => {
    button('Change…').click()
  })

  const snapshot = () => ({
    changeCalls: onRequestChange.mock.calls,
    restoredComment: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).value,
    storedDraftAfterFailure,
    storedDraftAfterReload: window.localStorage.getItem(storageKey),
  })

  expect(snapshot()).toEqual({
    changeCalls: [['Keep the request available for a retry.']],
    restoredComment: 'Keep the request available for a retry.',
    storedDraftAfterFailure: 'Keep the request available for a retry.',
    storedDraftAfterReload: 'Keep the request available for a retry.',
  })
})

it('sends the OK action from the expanded card', () => {
  const onOk = vi.fn(() => Promise.resolve(true))
  renderCard({ expanded: true, onOk })

  act(() => {
    button('OK').click()
  })

  expect(onOk.mock.calls).toEqual([[]])
})

it('keeps the conversation summary on one row at mobile widths', async () => {
  await page.viewport(390, 844)
  renderCard({
    decision: {
      ...decision,
      title: 'Keep retries within a predictable time budget',
    },
    position: '131 of 132',
  })

  const summary = requireElement(
    container.querySelector('.decision-card-summary'),
  )
  const title = requireElement(container.querySelector('.decision-card-title'))
  const consideredCount = requireElement(
    container.querySelector('.decision-card-considered-count'),
  )
  const mobileState = {
    consideredCountDisplay: getComputedStyle(consideredCount).display,
    summaryFlexWrap: getComputedStyle(summary).flexWrap,
    summaryFitsOneLine: summary.getBoundingClientRect().height < 48,
    titleHasSpace: title.getBoundingClientRect().width > 0,
  }

  await page.viewport(1280, 800)

  const responsiveState = () => ({
    mobile: mobileState,
    desktop: {
      consideredCountDisplay: getComputedStyle(consideredCount).display,
      consideredCountWhiteSpace: getComputedStyle(consideredCount).whiteSpace,
    },
  })

  expect(responsiveState()).toEqual({
    mobile: {
      consideredCountDisplay: 'none',
      summaryFlexWrap: 'nowrap',
      summaryFitsOneLine: true,
      titleHasSpace: true,
    },
    desktop: {
      consideredCountDisplay: 'block',
      consideredCountWhiteSpace: 'nowrap',
    },
  })
})

it('keeps the change target note below the panel actions when the name is long', () => {
  container.style.width = '280px'
  renderCard({
    variant: 'panel',
    expanded: true,
    targetName:
      'a long target session name for an asynchronous refactoring run',
  })

  const card = requireElement(container.querySelector('.decision-card'))
  const changeButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.decision-card-buttons [data-slot="button"]:last-child',
    ),
  )
  const targetNote = requireElement(
    container.querySelector('.decision-card-target-note'),
  )
  const shortcut = requireElement(changeButton.querySelector('kbd'))
  const cardBounds = card.getBoundingClientRect()
  const changeButtonBounds = changeButton.getBoundingClientRect()
  const shortcutBounds = shortcut.getBoundingClientRect()
  const getActual = () => ({
    changeButtonFitsCard: changeButtonBounds.right <= cardBounds.right,
    targetNoteText: targetNote.textContent,
    targetNoteFitsCard:
      targetNote.getBoundingClientRect().right <= cardBounds.right,
    shortcutFitsButton:
      shortcutBounds.width > 0 &&
      shortcutBounds.right <= changeButtonBounds.right,
  })
  expect(getActual()).toEqual({
    changeButtonFitsCard: true,
    targetNoteText:
      'Change… sends to a long target session name for an asynchronous refactoring run',
    targetNoteFitsCard: true,
    shortcutFitsButton: true,
  })
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
