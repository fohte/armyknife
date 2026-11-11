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
    onSkip: vi.fn(),
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

it('shows question metadata and sends skip and freeform actions', async () => {
  const onSkip = vi.fn()
  const onFreeformAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({
      header: 'Output format',
      question: 'Which format should be used?',
      choices: [{ label: 'Compact', description: 'Fits in a short report.' }],
      multiSelect: true,
      onSkip,
      onFreeformAnswer,
    }),
  )

  const skipButton = requireElement(
    Array.from(container.querySelectorAll('button')).find((button) =>
      button.textContent.includes('No answer'),
    ) ?? null,
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    skipButton.click()
    setTextareaValue(textarea, 'Use a fictional format')
  })
  await act(async () => {
    requireElement(container.querySelector('form')).dispatchEvent(
      new Event('submit', { bubbles: true, cancelable: true }),
    )
    await Promise.resolve()
  })

  const snapshot = () => ({
    heading: container.querySelector('.chat-question-card h2')?.textContent,
    multiSelect: container.querySelector('.chat-question-card p')?.textContent,
    description: container.querySelector('.chat-question-choice-description')
      ?.textContent,
    skipCalls: onSkip.mock.calls,
    answerCalls: onFreeformAnswer.mock.calls,
  })
  expect(snapshot()).toEqual({
    heading: 'Output format',
    multiSelect: 'Select all that apply.',
    description: 'Fits in a short report.',
    skipCalls: [[]],
    answerCalls: [['Use a fictional format']],
  })
})
