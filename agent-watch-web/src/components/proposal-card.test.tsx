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
    resolution: null,
    withdrawalComment: null,
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

function renderCards(cardProps: Props[]) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <>
        {cardProps.map((card) => (
          <ProposalCard {...card} key={card.seq} />
        ))}
      </>,
    )
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
      container.querySelectorAll<HTMLDivElement>('.chat-proposal-option'),
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

it('renders proposal summaries, descriptions, and withdrawal comments as markdown', () => {
  renderCard(
    props({
      summary: 'Compare **the options** in [the guide](https://example.test).',
      options: [
        {
          label: 'Use the existing flow',
          description:
            'Keep **the current flow** and [review details](https://example.test).',
          pros: [],
          cons: [],
          recommended: false,
        },
      ],
      resolved: true,
      resolution: 'withdrawn',
      withdrawalComment:
        'See [the update](https://example.test) for **context**.',
    }),
  )

  const snapshot = () => ({
    summary: container.querySelector('.chat-proposal-summary')?.innerHTML,
    description: container.querySelector('.chat-proposal-option-description')
      ?.innerHTML,
    withdrawalComment: container.querySelector(
      '.chat-proposal-withdrawal-comment',
    )?.innerHTML,
  })

  expect(snapshot()).toEqual({
    summary:
      '<p>Compare <strong>the options</strong> in <a href="https://example.test" target="_blank" rel="noopener noreferrer">the guide</a>.</p>',
    description:
      '<p>Keep <strong>the current flow</strong> and <a href="https://example.test" target="_blank" rel="noopener noreferrer">review details</a>.</p>',
    withdrawalComment:
      '<p>See <a href="https://example.test" target="_blank" rel="noopener noreferrer">the update</a> for <strong>context</strong>.</p>',
  })
})

it('keeps option labels phrasing-only and links associated with their radios', () => {
  renderCard(
    props({
      options: [
        {
          label: 'Use the existing flow',
          description: 'Read [the guide](https://example.test).',
          pros: [],
          cons: [],
          recommended: false,
        },
      ],
    }),
  )

  const snapshot = () => {
    const option = container.querySelector('.chat-proposal-option')
    const input = option?.querySelector<HTMLInputElement>('input')
    const label = option?.querySelector<HTMLLabelElement>('label')
    const description = option?.querySelector(
      '.chat-proposal-option-description',
    )

    return {
      optionTag: option?.tagName,
      inputId: input?.id,
      labelFor: label?.htmlFor,
      labelText: label?.textContent,
      labelLinkCount: label?.querySelectorAll('a').length,
      descriptionTag: description?.tagName,
      descriptionLink: description?.querySelector('a')?.getAttribute('href'),
    }
  }

  expect(snapshot()).toEqual({
    optionTag: 'DIV',
    inputId: 'proposal-7-option-0',
    labelFor: 'proposal-7-option-0',
    labelText: 'A · Use the existing flow',
    labelLinkCount: 0,
    descriptionTag: 'DIV',
    descriptionLink: 'https://example.test',
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

it('does not send Command-Enter when no choice is selected', () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderCard(props({ onAnswer, defaultComment: 'Add a simpler option.' }))
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const submitEvent = dispatchKey(comment, 'Enter', true)

  const snapshot = () => ({
    submitPrevented: submitEvent.defaultPrevented,
    answerCalls: onAnswer.mock.calls,
    status: container.querySelector('h2')?.textContent,
  })
  expect(snapshot()).toEqual({
    submitPrevented: false,
    answerCalls: [],
    status: 'Proposal · waiting for your choice',
  })
})

it('does not send Command-Enter from an input outside the proposal card', () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderCard(props({ onAnswer, defaultChoice: 1 }))
  const externalInput = document.createElement('input')
  document.body.append(externalInput)
  const submitEvent = dispatchKey(externalInput, 'Enter', true)
  externalInput.remove()

  const snapshot = () => ({
    submitPrevented: submitEvent.defaultPrevented,
    answerCalls: onAnswer.mock.calls,
    status: container.querySelector('h2')?.textContent,
  })
  expect(snapshot()).toEqual({
    submitPrevented: false,
    answerCalls: [],
    status: 'Proposal · waiting for your choice',
  })
})

it('sends Command-Enter to the proposal card containing the comment field', async () => {
  const firstAnswer = vi.fn(() => Promise.resolve(true))
  const secondAnswer = vi.fn(() => Promise.resolve(true))
  renderCards([
    props({ defaultChoice: 0, onAnswer: firstAnswer }),
    props({ seq: 8, defaultChoice: 1, onAnswer: secondAnswer }),
  ])
  const commentFields = container.querySelectorAll<HTMLTextAreaElement>(
    '.chat-proposal-comment',
  )
  const submitEvent = dispatchKey(commentFields[1] ?? window, 'Enter', true)
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    submitPrevented: submitEvent.defaultPrevented,
    firstCalls: firstAnswer.mock.calls,
    secondCalls: secondAnswer.mock.calls,
    statuses: Array.from(
      container.querySelectorAll('.chat-proposal-card h2'),
      (heading) => heading.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    submitPrevented: true,
    firstCalls: [],
    secondCalls: [[{ choice: 1, comment: null }]],
    statuses: ['Proposal · waiting for your choice', 'Proposal · chose B'],
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

it('shows a withdrawn proposal reason and disables every answer control', () => {
  renderCard(
    props({
      resolved: true,
      resolution: 'withdrawn',
      withdrawalComment: 'The direction changed.',
    }),
  )

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    comment: container.querySelector('.chat-proposal-withdrawal-comment')
      ?.textContent,
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
    status: 'Proposal · withdrawn',
    comment: 'The direction changed.',
    optionsDisabled: [true, true],
    buttonsDisabled: [true, true],
    commentDisabled: true,
  })
})
