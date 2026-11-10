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
    question: 'Which **retry policy** should be used?',
    choices: ['Retry *a few* times', 'Retry once'],
    answer: null,
    canAnswer: true,
    isMobile: false,
    showTerminalFallback: false,
    canOpenInTerminal: true,
    onAnswer: vi.fn(),
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
        check: '✓',
      },
      {
        disabled: true,
        selected: false,
        dimmed: true,
        textTag: 'SPAN',
        textMarkup: '<span>Retry once</span>',
        paragraphCount: 0,
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
      'Follow the [setup guide](https://example.org/guide)',
      'Use the default settings',
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
