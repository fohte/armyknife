import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { QuestionCard } from '#components/question-card'

type Props = ComponentProps<typeof QuestionCard>

let container: HTMLDivElement
let root: Root | null = null

function props(overrides: Partial<Props> = {}): Props {
  return {
    header: 'Retry policy',
    question: 'Which **retry policy** should be used?',
    choices: [
      { label: 'Retry *a few* times', description: 'Use a short delay.' },
      { label: 'Retry once' },
    ],
    multiSelect: false,
    answer: null,
    canAnswer: true,
    isMobile: false,
    showTerminalFallback: false,
    canOpenInTerminal: true,
    onAnswer: vi.fn(),
    onSkip: vi.fn(() => Promise.resolve(true)),
    onFreeformAnswer: vi.fn(() => Promise.resolve(true)),
    onOpenInTerminal: vi.fn(),
    ...overrides,
  }
}

function renderQuestion(cardProps: Props) {
  root ??= createRoot(container)
  act(() => {
    root?.render(<QuestionCard {...cardProps} />)
  })
}

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  const valueSetter = Reflect.getOwnPropertyDescriptor(
    textarea.constructor.prototype,
    'value',
  )?.set
  if (valueSetter !== undefined) {
    Reflect.apply(valueSetter, textarea, [value])
  }
  textarea.dispatchEvent(new Event('input', { bubbles: true }))
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

it('renders answered choice Markdown as inline spans without paragraphs', () => {
  renderQuestion(props({ answer: 'Retry *a few* times' }))

  const card = requireElement(container.querySelector('.chat-question-card'))
  const snapshot = () => ({
    question: card.querySelector('.chat-question-text strong')?.textContent,
    choices: Array.from(
      card.querySelectorAll<HTMLButtonElement>('.chat-question-choices button'),
      (button) => ({
        disabled: button.disabled,
        selected: button.classList.contains('is-selected'),
        dimmed: button.classList.contains('is-unselected'),
        textTag: button.querySelector('.chat-question-choice-text')?.tagName,
        textMarkup: button.querySelector('.chat-question-choice-text')
          ?.innerHTML,
        paragraphCount: button.querySelectorAll('p').length,
        description: button.querySelector('.chat-question-choice-description')
          ?.textContent,
        check: button.querySelector('.chat-question-choice-check')?.textContent,
      }),
    ),
    answer: card.querySelector('.chat-question-answer')?.textContent,
  })

  expect(snapshot()).toEqual({
    question: 'retry policy',
    choices: [
      {
        disabled: true,
        selected: true,
        dimmed: false,
        textTag: 'SPAN',
        textMarkup: '<span>Retry <em>a few</em> times</span>',
        paragraphCount: 0,
        description: 'Use a short delay.',
        check: '✓',
      },
      {
        disabled: true,
        selected: false,
        dimmed: true,
        textTag: 'SPAN',
        textMarkup: '<span>Retry once</span>',
        paragraphCount: 0,
        description: undefined,
        check: undefined,
      },
    ],
    answer: undefined,
  })
})

it('shows a link destination as text and answers only the selected choice', () => {
  const onAnswer = vi.fn()
  const cardProps = props({
    choices: [
      { label: 'Follow the [setup guide](https://example.org/guide)' },
      { label: 'Use the default settings' },
    ],
    onAnswer,
  })
  renderQuestion(cardProps)

  const destination = requireElement(
    container.querySelector('.chat-question-choice-text .markdown-link-text'),
  )
  act(() => {
    destination.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  const snapshot = () => ({
    destinationText: destination.textContent,
    destinationTag: destination.tagName,
    anchorCount: container.querySelectorAll('.chat-question-choices a').length,
    actionCalls: onAnswer.mock.calls,
  })

  expect(snapshot()).toEqual({
    destinationText: 'https://example.org/guide',
    destinationTag: 'SPAN',
    anchorCount: 0,
    actionCalls: [[1]],
  })
})

it('shows question metadata', () => {
  renderQuestion(
    props({
      header: 'Output format',
      question: 'Which format should be used?',
      choices: [{ label: 'Compact', description: 'Fits in a short report.' }],
      multiSelect: true,
    }),
  )

  const snapshot = () => ({
    heading: container.querySelector('.chat-question-card h2')?.textContent,
    multiSelect: container.querySelector('.chat-question-card p')?.textContent,
    description: container.querySelector('.chat-question-choice-description')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    heading: 'Output format',
    multiSelect: 'Multiple select question.',
    description: 'Fits in a short report.',
  })
  expect(container.querySelector('label')?.textContent).toBe('Type something')
})

it('skips a question from the no-answer button', async () => {
  const onSkip = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ onSkip }))

  const skipButton = requireElement(
    Array.from(container.querySelectorAll('button')).find((button) =>
      button.textContent.includes('No answer'),
    ) ?? null,
  )
  act(() => {
    skipButton.click()
  })
  await act(async () => Promise.resolve())

  expect(onSkip.mock.calls).toEqual([[]])
})

it('does not skip a question when Escape is pressed in the freeform field', async () => {
  const onSkip = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ onSkip }))
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const event = new KeyboardEvent('keydown', {
    key: 'Escape',
    bubbles: true,
    cancelable: true,
  })
  await act(async () => {
    textarea.dispatchEvent(event)
    await Promise.resolve()
  })

  const snapshot = () => ({
    skipCalls: onSkip.mock.calls,
    defaultPrevented: event.defaultPrevented,
  })
  expect(snapshot()).toEqual({ skipCalls: [], defaultPrevented: false })
})

it('skips a question when Escape is pressed elsewhere in the card', async () => {
  const onSkip = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ onSkip }))
  const card = requireElement(container.querySelector('.chat-question-card'))
  const event = new KeyboardEvent('keydown', {
    key: 'Escape',
    bubbles: true,
    cancelable: true,
  })
  await act(async () => {
    card.dispatchEvent(event)
    await Promise.resolve()
  })

  const snapshot = () => ({
    skipCalls: onSkip.mock.calls,
    defaultPrevented: event.defaultPrevented,
  })
  expect(snapshot()).toEqual({ skipCalls: [[]], defaultPrevented: true })
})

it('keeps a freeform answer available to retry after the question closes', async () => {
  const events: string[] = []
  const onSkip = vi.fn(() => {
    events.push('skip')
    return Promise.resolve(true)
  })
  let messageAttempts = 0
  const onFreeformAnswer = vi.fn(() => {
    events.push('message')
    messageAttempts += 1
    return Promise.resolve(messageAttempts > 1)
  })
  const cardProps = props({ onSkip, onFreeformAnswer })
  renderQuestion(cardProps)

  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    setTextareaValue(textarea, 'Use a fictional format')
  })
  await act(async () => {
    requireElement(container.querySelector('form')).dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    )
    await Promise.resolve()
    await Promise.resolve()
  })

  renderQuestion(props({ ...cardProps, canAnswer: false }))
  const retryTextarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const retryButton = requireElement(
    container.querySelector<HTMLButtonElement>('button[type="submit"]'),
  )
  const failedSendSnapshot = {
    draft: retryTextarea.value,
    retryDisabled: retryButton.disabled,
    error: container.querySelector('[role="alert"]')?.textContent,
    events: [...events],
  }

  await act(async () => {
    requireElement(container.querySelector('form')).dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    )
    await Promise.resolve()
  })

  const snapshot = () => ({
    failedSendSnapshot,
    errorAfterRetry: container.querySelector('[role="alert"]')?.textContent,
    events,
    skipCalls: onSkip.mock.calls,
    answerCalls: onFreeformAnswer.mock.calls,
  })
  expect(snapshot()).toEqual({
    failedSendSnapshot: {
      draft: 'Use a fictional format',
      retryDisabled: false,
      error:
        'Your answer was not sent. Your draft is still available; select Send answer to retry.',
      events: ['skip', 'message'],
    },
    errorAfterRetry: undefined,
    events: ['skip', 'message', 'message'],
    skipCalls: [[]],
    answerCalls: [['Use a fictional format'], ['Use a fictional format']],
  })
})
