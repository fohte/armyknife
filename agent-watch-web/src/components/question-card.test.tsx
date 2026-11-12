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
    questions: [
      {
        header: 'Colors',
        question: 'Which colors should be included?',
        choices: [{ label: 'Cedar' }, { label: 'Amber' }, { label: 'Jade' }],
        multi_select: true,
        answer: null,
      },
      {
        header: 'Icon',
        question: 'Which icon?',
        choices: [{ label: 'Compass' }, { label: 'Lantern' }],
        multi_select: false,
        answer: null,
      },
    ],
    canAnswer: true,
    showTerminalFallback: false,
    canOpenInTerminal: true,
    onAnswer: vi.fn(() => Promise.resolve(true)),
    onSkip: vi.fn(() => Promise.resolve(true)),
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

function choiceButton(label: string): HTMLButtonElement {
  return requireElement(
    Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
    ).find((button) => button.textContent.includes(label)) ?? null,
  )
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

it('submits every selected answer once with multi-select labels joined', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ onAnswer }))

  const submit = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-question-actions button.aw-button--primary',
    ),
  )
  const beforeAllQuestionsAreAnswered = submit.disabled
  act(() => {
    choiceButton('Cedar').click()
    choiceButton('Amber').click()
  })
  const afterFirstQuestionIsAnswered = submit.disabled
  act(() => {
    choiceButton('Compass').click()
  })
  const afterEveryQuestionIsAnswered = submit.disabled

  await act(async () => {
    submit.click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    disabled: [
      beforeAllQuestionsAreAnswered,
      afterFirstQuestionIsAnswered,
      afterEveryQuestionIsAnswered,
    ],
    answers: onAnswer.mock.calls,
    submitAfterAnswer: submit.disabled,
  })
  expect(snapshot()).toEqual({
    disabled: [true, true, false],
    answers: [
      [
        {
          'Which colors should be included?': 'Cedar, Amber',
          'Which icon?': 'Compass',
        },
      ],
    ],
    submitAfterAnswer: true,
  })
})

it('submits freeform answers for every question', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  const onSkip = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ onAnswer, onSkip }))

  const textareas = Array.from(
    container.querySelectorAll<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    for (const [index, textarea] of textareas.entries()) {
      const setter = Reflect.getOwnPropertyDescriptor(
        textarea.constructor.prototype,
        'value',
      )?.set
      if (setter !== undefined) {
        Reflect.apply(setter, textarea, [`Custom ${String(index)}`])
      }
      textarea.dispatchEvent(new Event('input', { bubbles: true }))
    }
  })

  const submit = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-question-actions button.aw-button--primary',
    ),
  )
  await act(async () => {
    submit.click()
    await Promise.resolve()
  })

  expect(onAnswer.mock.calls).toEqual([
    [
      {
        'Which colors should be included?': 'Custom 0',
        'Which icon?': 'Custom 1',
      },
    ],
  ])
})

it('keeps the answer after a failed submission and allows retry', async () => {
  const onAnswer = vi
    .fn<() => Promise<boolean>>()
    .mockResolvedValueOnce(false)
    .mockResolvedValueOnce(true)
  renderQuestion(props({ onAnswer }))
  act(() => {
    choiceButton('Cedar').click()
    choiceButton('Compass').click()
  })
  const submit = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-question-actions button.aw-button--primary',
    ),
  )

  await act(async () => {
    submit.click()
    await Promise.resolve()
    await Promise.resolve()
  })

  const afterFailedSubmission = {
    error: container.querySelector('[role="alert"]')?.textContent,
    cedarSelected: choiceButton('Cedar').getAttribute('aria-pressed'),
    submitDisabled: submit.disabled,
  }

  await act(async () => {
    submit.click()
    await Promise.resolve()
    await Promise.resolve()
  })

  const snapshot = () => ({
    afterFailedSubmission,
    calls: onAnswer.mock.calls,
    errorAfterRetry: container.querySelector('[role="alert"]'),
    submitAfterRetry: container.querySelector(
      '.chat-question-actions button.aw-button--primary',
    ),
  })
  expect(snapshot()).toEqual({
    afterFailedSubmission: {
      error:
        'Your answer was not sent. Your draft is still available; select Send answer to retry.',
      cedarSelected: 'true',
      submitDisabled: false,
    },
    calls: [
      [
        {
          'Which colors should be included?': 'Cedar',
          'Which icon?': 'Compass',
        },
      ],
      [
        {
          'Which colors should be included?': 'Cedar',
          'Which icon?': 'Compass',
        },
      ],
    ],
    errorAfterRetry: null,
    submitAfterRetry: null,
  })
})

it('skips all questions with one request', async () => {
  const onSkip = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ onSkip }))
  const skip = requireElement(
    Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent.includes('No answer'),
    ) ?? null,
  )

  await act(async () => {
    skip.click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    skipCalls: onSkip.mock.calls,
    skipButtonPresent:
      container.querySelector(
        '.chat-question-actions button.aw-button--outline',
      ) !== null,
  })
  expect(snapshot()).toEqual({ skipCalls: [[]], skipButtonPresent: false })
})

it('shows the terminal fallback for questions without a watch-web request', () => {
  renderQuestion(
    props({
      canAnswer: false,
      showTerminalFallback: true,
      canOpenInTerminal: false,
    }),
  )

  const snapshot = () => ({
    terminalMessage: container.querySelector('.conversation-terminal-fallback')
      ?.textContent,
    submitButton: container.querySelector(
      '.chat-question-actions button.aw-button--primary',
    ),
    promptCount: container.querySelectorAll('.chat-question-prompt').length,
  })
  expect(snapshot()).toEqual({
    terminalMessage: 'Answer in terminal to continue.',
    submitButton: null,
    promptCount: 2,
  })
})
