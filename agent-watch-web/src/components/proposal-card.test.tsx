import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ProposalCard } from '#components/proposal-card'

type Props = ComponentProps<typeof ProposalCard>

let container: HTMLDivElement
let root: Root | null = null

function props(overrides: Partial<Props> = {}): Props {
  return {
    seq: 7,
    title: 'When should the setting take effect?',
    summary: 'Choose how the change is introduced.',
    options: [
      {
        label: 'Use the existing flow',
        description: 'Explain the change before applying it.',
        pros: ['Small change'],
        cons: ['Adds one step'],
        recommended: true,
      },
      {
        label: 'Show the choice earlier',
        description: 'Move the choice before the next action.',
        pros: ['Clearer timing'],
        cons: [],
        recommended: false,
      },
    ],
    answer: null,
    resolved: false,
    isMobile: false,
    onAnswer: vi.fn(() => Promise.resolve(true)),
    ...overrides,
  }
}

function renderCard(cardProps: Props) {
  root ??= createRoot(container)
  act(() => {
    root?.render(<ProposalCard {...cardProps} />)
  })
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
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, [value])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function dispatchKey(target: EventTarget, key: string, metaKey = false) {
  const event = new KeyboardEvent('keydown', {
    key,
    metaKey,
    bubbles: true,
    cancelable: true,
  })
  act(() => {
    target.dispatchEvent(event)
  })
  return event
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

it('renders the proposal heading, recommended option, and pros and cons', () => {
  renderCard(props())

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    title: container.querySelector('h3')?.textContent,
    summary: container.querySelector('.chat-proposal-summary')?.textContent,
    groupRole: container
      .querySelector('.chat-proposal-options')
      ?.getAttribute('role'),
    options: Array.from(
      container.querySelectorAll<HTMLLabelElement>('.chat-proposal-option'),
      (option) => ({
        text: option.textContent,
        checked: option.querySelector<HTMLInputElement>('input')?.checked,
        recommended: option.querySelector('.chat-proposal-recommended')
          ?.textContent,
      }),
    ),
    buttons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-proposal-actions button',
      ),
      (button) => ({ text: button.textContent, disabled: button.disabled }),
    ),
    hint: container.querySelector('.chat-proposal-hint')?.textContent,
  })

  expect(snapshot()).toEqual({
    status: 'Proposal · waiting for your choice',
    title: 'When should the setting take effect?',
    summary: 'Choose how the change is introduced.',
    groupRole: 'radiogroup',
    options: [
      {
        text: 'A · Use the existing flowRecommendedExplain the change before applying it.+ Small change− Adds one step',
        checked: false,
        recommended: 'Recommended',
      },
      {
        text: 'B · Show the choice earlierMove the choice before the next action.+ Clearer timing',
        checked: false,
        recommended: undefined,
      },
    ],
    buttons: [
      { text: 'Choose …', disabled: true },
      { text: 'Ask for another option', disabled: false },
    ],
    hint: '1 / 2 to pick · ⌘↵ to send',
  })
})

it('selects a numbered option and sends it with the comment on Command-Enter', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderCard(props({ onAnswer }))
  dispatchKey(window, '2')
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  setTextareaValue(comment, 'Keep the earlier prompt short.')
  const submitEvent = dispatchKey(comment, 'Enter', true)
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    submitPrevented: submitEvent.defaultPrevented,
    answerCalls: onAnswer.mock.calls,
    status: container.querySelector('h2')?.textContent,
    selected: Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      ),
      (input) => input.checked,
    ),
    disabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-proposal-actions button',
      ),
      (button) => button.disabled,
    ),
    comment: comment.value,
  })

  expect(snapshot()).toEqual({
    submitPrevented: true,
    answerCalls: [[{ choice: 1, comment: 'Keep the earlier prompt short.' }]],
    status: 'Proposal · chose B',
    selected: [false, true],
    disabled: [true, true],
    comment: 'Keep the earlier prompt short.',
  })
})

it('asks for another option with a null choice and an optional comment', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderCard(props({ onAnswer, defaultComment: 'Add a simpler alternative.' }))
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-proposal-actions button:last-child',
      ),
    ).click()
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    answerCalls: onAnswer.mock.calls,
    status: container.querySelector('h2')?.textContent,
    optionsDisabled: Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      ),
      (input) => input.disabled,
    ),
  })
  expect(snapshot()).toEqual({
    answerCalls: [[{ choice: null, comment: 'Add a simpler alternative.' }]],
    status: 'Proposal · asked for another option',
    optionsDisabled: [true, true],
  })
})

it('disables choices when the proposal was answered elsewhere', () => {
  renderCard(props({ resolved: true }))

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    optionsDisabled: Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      ),
      (input) => input.disabled,
    ),
    buttonsDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-proposal-actions button',
      ),
      (button) => button.disabled,
    ),
    commentDisabled: container.querySelector<HTMLTextAreaElement>(
      '.chat-proposal-comment',
    )?.disabled,
  })

  expect(snapshot()).toEqual({
    status: 'Proposal · answered elsewhere',
    optionsDisabled: [true, true],
    buttonsDisabled: [true, true],
    commentDisabled: true,
  })
})
