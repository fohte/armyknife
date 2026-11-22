import '#index.css'

import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import { ProposalCard } from '#components/proposal-card'
import { QuestionCard } from '#components/question-card'
import type { SessionLinkTarget } from '#types'

type Props = ComponentProps<typeof QuestionCard>

const linkedSessionId = '11111111-1111-4111-8111-111111111111'
const linkedSession: SessionLinkTarget = {
  id: linkedSessionId,
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}
const onSelectSession = vi.fn<(id: string) => void>()

const singleChoiceQuestions: Props['questions'] = [
  {
    header: 'Direction',
    question: 'Which direction should be used?',
    choices: [{ label: 'North' }, { label: 'South' }, { label: 'East' }],
    multi_select: false,
    answer: null,
  },
]

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
    isMobile: false,
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
    root?.render(
      <MarkdownSessionLinksProvider
        sessions={[linkedSession]}
        onSelectSession={onSelectSession}
      >
        <QuestionCard {...cardProps} />
      </MarkdownSessionLinksProvider>,
    )
  })
}

function renderQuestions(cardProps: Props[]) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <>
        {cardProps.map((questionProps, index) => (
          <QuestionCard {...questionProps} key={String(index)} />
        ))}
      </>,
    )
  })
}

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function choiceButton(label: string): HTMLButtonElement {
  const row = Array.from(
    container.querySelectorAll<HTMLElement>('.chat-question-choice-row'),
  ).find((choiceRow) => {
    const choiceText = choiceRow.querySelector('.chat-question-choice-text')
    return choiceText !== null && choiceText.textContent.includes(label)
  })
  return requireElement(row?.querySelector<HTMLButtonElement>('button') ?? null)
}

function dispatchKey(target: EventTarget, key: string) {
  const event = new KeyboardEvent('keydown', {
    key,
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

it('links session IDs in question prompts, choice labels, and descriptions', () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({
      onAnswer,
      questions: [
        {
          header: 'Session',
          question: `Should this follow ${linkedSessionId}?`,
          choices: [
            {
              label: `Open ${linkedSessionId}`,
              description: `Read the context in ${linkedSessionId}.`,
            },
            { label: 'Use another approach' },
          ],
          multi_select: false,
          answer: null,
        },
      ],
    }),
  )

  const links = Array.from(
    container.querySelectorAll<HTMLAnchorElement>('.chat-question-card a'),
  )
  act(() => {
    links.forEach((link) => {
      link.dispatchEvent(
        new MouseEvent('click', { bubbles: true, cancelable: true }),
      )
    })
  })

  const getActual = () => ({
    links: links.map((link) => ({
      text: link.textContent,
      href: link.getAttribute('href'),
    })),
    selectedSessions: onSelectSession.mock.calls.map(([id]) => id),
    answers: onAnswer.mock.calls,
  })
  expect(getActual()).toEqual({
    links: [
      {
        text: linkedSessionId,
        href: `?session=${linkedSessionId}`,
      },
      {
        text: linkedSessionId,
        href: `?session=${linkedSessionId}`,
      },
      {
        text: linkedSessionId,
        href: `?session=${linkedSessionId}`,
      },
    ],
    selectedSessions: [linkedSessionId, linkedSessionId, linkedSessionId],
    answers: [],
  })
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

it('sends a single-choice answer as soon as an option is selected', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({ questions: singleChoiceQuestions, isMobile: true, onAnswer }),
  )

  await act(async () => {
    choiceButton('South').click()
    await Promise.resolve()
    await Promise.resolve()
  })

  const snapshot = () => ({
    answerCalls: onAnswer.mock.calls,
    selected: choiceButton('South').getAttribute('aria-pressed'),
    submitButton: container.querySelector(
      '.chat-question-actions button.aw-button--primary',
    ),
  })
  expect(snapshot()).toEqual({
    answerCalls: [[{ 'Which direction should be used?': 'South' }]],
    selected: 'true',
    submitButton: null,
  })
})

it('selects a single-choice option when its description is clicked', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({
      onAnswer,
      questions: [
        {
          header: 'Direction',
          question: 'Which direction should be used?',
          choices: [{ label: 'North', description: 'Follow the north path.' }],
          multi_select: false,
          answer: null,
        },
      ],
    }),
  )

  const description = requireElement(
    container.querySelector<HTMLElement>('.chat-question-choice-description'),
  )
  await act(async () => {
    description.click()
    await Promise.resolve()
    await Promise.resolve()
  })

  const getActual = () => ({
    answerCalls: onAnswer.mock.calls,
    selected: choiceButton('North').getAttribute('aria-pressed'),
  })
  expect(getActual()).toEqual({
    answerCalls: [[{ 'Which direction should be used?': 'North' }]],
    selected: 'true',
  })
})

it('describes a single-choice answer button with its description', () => {
  renderQuestion(
    props({
      questions: [
        {
          header: 'Direction',
          question: 'Which direction should be used?',
          choices: [{ label: 'North', description: 'Follow the north path.' }],
          multi_select: false,
          answer: null,
        },
      ],
    }),
  )

  const button = choiceButton('North')
  const title = requireElement(
    container.querySelector<HTMLElement>('.chat-question-choice-content'),
  )
  const description = requireElement(
    container.querySelector<HTMLElement>('.chat-question-choice-description'),
  )
  const getActual = () => ({
    buttonDescribesChoice:
      button.getAttribute('aria-describedby') === description.id,
    buttonNamesChoice: button.getAttribute('aria-labelledby') === title.id,
    descriptionHasId: description.id.length > 0,
    titleHasId: title.id.length > 0,
  })
  expect(getActual()).toEqual({
    buttonDescribesChoice: true,
    buttonNamesChoice: true,
    descriptionHasId: true,
    titleHasId: true,
  })
})

it('does not answer a single-choice question when copying code from its label', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({
      onAnswer,
      questions: [
        {
          header: 'Direction',
          question: 'Which direction should be used?',
          choices: [{ label: 'North\n\n```text\nnorth\n```' }],
          multi_select: false,
          answer: null,
        },
      ],
    }),
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-question-choice-content .syntax-code-block-copy-button',
    ),
  )
  await act(async () => {
    copyButton.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const getActual = () => ({
    answerCalls: onAnswer.mock.calls,
    copyButtonNestedInChoice: choiceButton('North').contains(copyButton),
    selected: choiceButton('North').getAttribute('aria-pressed'),
  })
  expect(getActual()).toEqual({
    answerCalls: [],
    copyButtonNestedInChoice: false,
    selected: 'false',
  })
})

it('renders copy controls outside a single-choice option description button', () => {
  renderQuestion(
    props({
      questions: [
        {
          header: 'Direction',
          question: 'Which direction should be used?',
          choices: [
            {
              label: 'North',
              description: '```text\nnorth\n```',
            },
          ],
          multi_select: false,
          answer: null,
        },
      ],
      isMobile: true,
    }),
  )

  const snapshot = () => ({
    copyButtonRendered:
      container.querySelector('.syntax-code-block-copy-button') !== null,
    copyButtonNestedInChoice:
      choiceButton('North').querySelector('.syntax-code-block-copy-button') !==
      null,
  })
  expect(snapshot()).toEqual({
    copyButtonRendered: true,
    copyButtonNestedInChoice: false,
  })
})

it('frames a recorded choice with its description without doubling dividers', () => {
  renderQuestion(
    props({
      canAnswer: false,
      questions: [
        {
          header: 'Retry limit',
          question: 'How many attempts should run?',
          choices: [
            {
              label: 'Three attempts',
              description: 'Stop after three failures.',
            },
            { label: 'Five attempts', description: 'Continue for longer.' },
          ],
          multi_select: false,
          answer: 'Three attempts',
        },
      ],
    }),
  )

  const selectedRow = requireElement(
    container.querySelector<HTMLElement>(
      '.chat-question-choice-row.is-selected',
    ),
  )
  const choices = requireElement(
    container.querySelector<HTMLElement>('.chat-question-choices'),
  )
  const unselectedRow = requireElement(
    choices.querySelector<HTMLElement>(
      '.chat-question-choice-row:not(.is-selected)',
    ),
  )
  const selectedButton = requireElement(
    selectedRow.querySelector<HTMLButtonElement>('button'),
  )
  const description = requireElement(
    selectedRow.querySelector<HTMLElement>('.chat-question-choice-description'),
  )
  const descriptionParagraph = requireElement(
    description.querySelector<HTMLParagraphElement>('p'),
  )
  const choiceContent = requireElement(
    selectedRow.querySelector<HTMLElement>('.chat-question-choice-content'),
  )
  const rowStyle = getComputedStyle(selectedRow)
  const choicesStyle = getComputedStyle(choices)
  const unselectedRowStyle = getComputedStyle(unselectedRow)
  const buttonStyle = getComputedStyle(selectedButton)
  const choiceContentStyle = getComputedStyle(choiceContent)
  const paragraphStyle = getComputedStyle(descriptionParagraph)

  const getActual = () => ({
    rowBorder: [
      rowStyle.borderTopWidth,
      rowStyle.borderRightWidth,
      rowStyle.borderBottomWidth,
      rowStyle.borderLeftWidth,
    ],
    rowBackground: rowStyle.backgroundColor,
    choicesBorderTop: choicesStyle.borderTopWidth,
    unselectedRowBorderTop: unselectedRowStyle.borderTopWidth,
    buttonBorder: [
      buttonStyle.borderTopWidth,
      buttonStyle.borderRightWidth,
      buttonStyle.borderBottomWidth,
      buttonStyle.borderLeftWidth,
    ],
    buttonBackground: buttonStyle.backgroundColor,
    buttonMinHeight: buttonStyle.minHeight,
    choiceContentPaddingBottom: choiceContentStyle.paddingBottom,
    descriptionParagraph: {
      margin: paragraphStyle.margin,
      color: paragraphStyle.color,
      fontSize: paragraphStyle.fontSize,
    },
    descriptionInsideSelectedRow: description.parentElement === selectedRow,
  })
  expect(getActual()).toEqual({
    rowBorder: ['1px', '1px', '1px', '1px'],
    rowBackground: 'rgb(20, 20, 20)',
    choicesBorderTop: '0px',
    unselectedRowBorderTop: '0px',
    buttonBorder: ['0px', '0px', '0px', '0px'],
    buttonBackground: 'rgba(0, 0, 0, 0)',
    buttonMinHeight: '0px',
    choiceContentPaddingBottom: '0px',
    descriptionParagraph: {
      margin: '0px',
      color: 'rgb(161, 161, 170)',
      fontSize: '11.5px',
    },
    descriptionInsideSelectedRow: true,
  })
})

it('keeps descriptions compact on unanswered choices', () => {
  renderQuestion(
    props({
      questions: [
        {
          header: 'Retry limit',
          question: 'How many attempts should run?',
          choices: [
            {
              label: 'Three attempts',
              description: 'Stop after three failures.',
            },
            { label: 'Five attempts' },
          ],
          multi_select: false,
          answer: null,
        },
      ],
    }),
  )

  const row = requireElement(
    container.querySelector<HTMLElement>(
      '.chat-question-choice-row.has-description.has-shortcut:not(.is-selected)',
    ),
  )
  const button = requireElement(row.querySelector<HTMLButtonElement>('button'))
  const descriptionParagraph = requireElement(
    row.querySelector<HTMLParagraphElement>(
      '.chat-question-choice-description p',
    ),
  )
  const choiceTitle = requireElement(
    row.querySelector<HTMLElement>('.chat-question-choice-text'),
  )
  const choiceContent = requireElement(
    row.querySelector<HTMLElement>('.chat-question-choice-content'),
  )
  const buttonStyle = getComputedStyle(button)
  const choiceContentStyle = getComputedStyle(choiceContent)
  const paragraphStyle = getComputedStyle(descriptionParagraph)
  const choiceTitleRect = choiceTitle.getBoundingClientRect()
  const descriptionRect = descriptionParagraph.getBoundingClientRect()

  const getActual = () => ({
    answerState: button.getAttribute('aria-pressed'),
    buttonMinHeight: buttonStyle.minHeight,
    contentAlignItems: choiceContentStyle.alignItems,
    choiceContentPaddingBottom: choiceContentStyle.paddingBottom,
    alignmentOffset: Number(
      (descriptionRect.left - choiceTitleRect.left).toFixed(2),
    ),
    descriptionParagraph: {
      margin: paragraphStyle.margin,
      color: paragraphStyle.color,
      fontSize: paragraphStyle.fontSize,
    },
  })
  expect(getActual()).toEqual({
    answerState: 'false',
    buttonMinHeight: '0px',
    contentAlignItems: 'baseline',
    choiceContentPaddingBottom: '0px',
    alignmentOffset: 0,
    descriptionParagraph: {
      margin: '0px',
      color: 'rgb(161, 161, 170)',
      fontSize: '11.5px',
    },
  })
})

it('answers a desktop single-choice question with its number shortcut', async () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ questions: singleChoiceQuestions, onAnswer }))
  const shortcuts = Array.from(
    container.querySelectorAll('.chat-question-choices kbd'),
    (key) => key.textContent,
  )

  const event = dispatchKey(window, '2')
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })

  const snapshot = () => ({
    shortcuts,
    answerCalls: onAnswer.mock.calls,
    selected: choiceButton('South').getAttribute('aria-pressed'),
    defaultPrevented: event.defaultPrevented,
  })
  expect(snapshot()).toEqual({
    shortcuts: ['1', '2', '3'],
    answerCalls: [[{ 'Which direction should be used?': 'South' }]],
    selected: 'true',
    defaultPrevented: true,
  })
})

it('does not answer a desktop question while its answer field is focused', () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(props({ questions: singleChoiceQuestions, onAnswer }))
  const answerField = requireElement(container.querySelector('textarea'))

  const event = dispatchKey(answerField, '2')

  const snapshot = () => ({
    answerCalls: onAnswer.mock.calls,
    selected: choiceButton('South').getAttribute('aria-pressed'),
    defaultPrevented: event.defaultPrevented,
  })
  expect(snapshot()).toEqual({
    answerCalls: [],
    selected: 'false',
    defaultPrevented: false,
  })
})

it('routes shortcuts to the latest question card or the card containing focus', async () => {
  const firstAnswer = vi.fn(() => Promise.resolve(true))
  const lastAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestions([
    props({ questions: singleChoiceQuestions, onAnswer: firstAnswer }),
    props({ questions: singleChoiceQuestions, onAnswer: lastAnswer }),
  ])

  const outsideEvent = dispatchKey(window, '2')
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
  const afterOutsideShortcut = {
    firstCalls: firstAnswer.mock.calls.slice(),
    lastCalls: lastAnswer.mock.calls.slice(),
    selectedPerCard: Array.from(
      container.querySelectorAll('.chat-question-card'),
      (card) =>
        card.querySelectorAll(
          '.chat-question-choices button[aria-pressed="true"]',
        ).length,
    ),
    defaultPrevented: outsideEvent.defaultPrevented,
  }

  const insideEvent = dispatchKey(choiceButton('North'), '1')
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })

  const snapshot = () => ({
    afterOutsideShortcut,
    insideDefaultPrevented: insideEvent.defaultPrevented,
    firstCalls: firstAnswer.mock.calls,
    lastCalls: lastAnswer.mock.calls,
    selectedPerCard: Array.from(
      container.querySelectorAll('.chat-question-card'),
      (card) =>
        card.querySelectorAll(
          '.chat-question-choices button[aria-pressed="true"]',
        ).length,
    ),
  })
  expect(snapshot()).toEqual({
    afterOutsideShortcut: {
      firstCalls: [],
      lastCalls: [[{ 'Which direction should be used?': 'South' }]],
      selectedPerCard: [0, 1],
      defaultPrevented: true,
    },
    insideDefaultPrevented: true,
    firstCalls: [[{ 'Which direction should be used?': 'North' }]],
    lastCalls: [[{ 'Which direction should be used?': 'South' }]],
    selectedPerCard: [1, 1],
  })
})

it('leaves a number shortcut to the proposal card that owns the target', () => {
  const onQuestionAnswer = vi.fn(() => Promise.resolve(true))
  const onProposalAnswer = vi.fn(() => Promise.resolve(true))
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <>
        <QuestionCard
          {...props({
            questions: singleChoiceQuestions,
            onAnswer: onQuestionAnswer,
          })}
        />
        <ProposalCard
          seq={9}
          title="Which route should be used?"
          summary={null}
          options={[
            {
              label: 'North route',
              description: null,
              pros: [],
              cons: [],
              recommended: false,
            },
            {
              label: 'South route',
              description: null,
              pros: [],
              cons: [],
              recommended: false,
            },
          ]}
          answer={null}
          resolved={false}
          resolution={null}
          withdrawalComment={null}
          isMobile={false}
          onAnswer={onProposalAnswer}
        />
      </>,
    )
  })
  const proposalAction = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-proposal-actions button.aw-button--outline',
    ),
  )

  const event = dispatchKey(proposalAction, '1')

  const snapshot = () => ({
    questionCalls: onQuestionAnswer.mock.calls,
    proposalCalls: onProposalAnswer.mock.calls,
    selectedProposalOptions: Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      ),
      (input) => input.checked,
    ),
    defaultPrevented: event.defaultPrevented,
  })
  expect(snapshot()).toEqual({
    questionCalls: [],
    proposalCalls: [],
    selectedProposalOptions: [true, false],
    defaultPrevented: true,
  })
})

it('does not show or use number shortcuts on mobile', () => {
  const onAnswer = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({ questions: singleChoiceQuestions, isMobile: true, onAnswer }),
  )

  const event = dispatchKey(window, '2')

  const snapshot = () => ({
    shortcuts: container.querySelectorAll('.chat-question-choices kbd').length,
    answerCalls: onAnswer.mock.calls,
    defaultPrevented: event.defaultPrevented,
  })
  expect(snapshot()).toEqual({
    shortcuts: 0,
    answerCalls: [],
    defaultPrevented: false,
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

it('restores question answers after reload and clears them after sending', async () => {
  const sessionId = crypto.randomUUID()
  const requestId = 'fixture-question-request'
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'question',
    sessionId,
    requestId,
    'answers',
  ])}`
  const selectionStorageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'question',
    sessionId,
    requestId,
    'selections',
  ])}`
  const onAnswer = vi.fn(() => Promise.resolve(true))
  const cardProps = props({
    sessionId,
    requestId,
    isMobile: true,
    onAnswer,
  })
  renderQuestion(cardProps)
  act(() => {
    choiceButton('Cedar').click()
  })
  await act(async () => {
    await userEvent.fill(
      requireElement(
        container.querySelectorAll<HTMLTextAreaElement>('textarea').item(1),
      ),
      'Choose the icon with a clear silhouette.',
    )
  })
  const storedDraftBeforeReload = window.localStorage.getItem(storageKey)
  const storedSelectionBeforeReload =
    window.localStorage.getItem(selectionStorageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderQuestion(cardProps)

  const restoredAnswer = requireElement(
    container.querySelectorAll<HTMLTextAreaElement>('textarea').item(1),
  )
  const persistedBeforeSend = restoredAnswer.value
  const restoredChoice = choiceButton('Cedar').getAttribute('aria-pressed')
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-question-actions button.aw-button--primary',
      ),
    ).click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    restoredAnswer: persistedBeforeSend,
    restoredChoice,
    answerCalls: onAnswer.mock.calls,
    storedDraftBeforeReload,
    storedSelectionBeforeReload,
    storedDraft: window.localStorage.getItem(storageKey),
    storedSelection: window.localStorage.getItem(selectionStorageKey),
  })

  expect(snapshot()).toEqual({
    restoredAnswer: 'Choose the icon with a clear silhouette.',
    restoredChoice: 'true',
    answerCalls: [
      [
        {
          'Which colors should be included?': 'Cedar',
          'Which icon?': 'Choose the icon with a clear silhouette.',
        },
      ],
    ],
    storedDraftBeforeReload:
      'Which+icon%3F=Choose+the+icon+with+a+clear+silhouette.',
    storedSelectionBeforeReload: 'Which+colors+should+be+included%3F=Cedar',
    storedDraft: null,
    storedSelection: null,
  })
})

it('keeps freeform question answers after an unsuccessful send and reload', async () => {
  const sessionId = crypto.randomUUID()
  const requestId = 'fixture-retry-question'
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'question',
    sessionId,
    requestId,
    'answers',
  ])}`
  const onAnswer = vi.fn(() => Promise.resolve(false))
  const cardProps = props({
    sessionId,
    requestId,
    questions: singleChoiceQuestions,
    isMobile: true,
    onAnswer,
  })
  renderQuestion(cardProps)
  await act(async () => {
    await userEvent.fill(
      requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
      'Use the lower impact option.',
    )
  })
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-question-actions button.aw-button--primary',
      ),
    ).click()
    await Promise.resolve()
  })
  const storedDraftAfterFailure = window.localStorage.getItem(storageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderQuestion(cardProps)

  const snapshot = () => ({
    answerCalls: onAnswer.mock.calls,
    restoredAnswer: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).value,
    storedDraftAfterFailure,
    storedDraftAfterReload: window.localStorage.getItem(storageKey),
  })

  expect(snapshot()).toEqual({
    answerCalls: [
      [{ 'Which direction should be used?': 'Use the lower impact option.' }],
    ],
    restoredAnswer: 'Use the lower impact option.',
    storedDraftAfterFailure:
      'Which+direction+should+be+used%3F=Use+the+lower+impact+option.',
    storedDraftAfterReload:
      'Which+direction+should+be+used%3F=Use+the+lower+impact+option.',
  })
})

it('warns when browser storage cannot save a question draft', async () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new DOMException('Storage is blocked.')
  })
  renderQuestion(
    props({
      sessionId: 'fixture-storage-session',
      requestId: 'fixture-storage-question',
      questions: singleChoiceQuestions,
      isMobile: true,
    }),
  )
  await act(async () => {
    await userEvent.fill(
      requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
      'Use the stable option.',
    )
  })

  const snapshot = () => ({
    answer: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).value,
    notice: container.querySelector('[role="status"]')?.textContent,
  })

  expect(snapshot()).toEqual({
    answer: 'Use the stable option.',
    notice:
      'Browser storage is unavailable. Drafts may not survive a page reload.',
  })
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

it('closes a question image lightbox with Escape without skipping the question', async () => {
  const onSkip = vi.fn(() => Promise.resolve(true))
  renderQuestion(
    props({
      onSkip,
      questions: [
        {
          header: 'Review the image',
          question:
            '![Fixture chart](https://example.invalid/fixture-chart.png)',
          choices: [{ label: 'Continue' }, { label: 'Stop' }],
          multi_select: false,
          answer: null,
        },
      ],
    }),
  )

  const imageTrigger = requireElement(
    container.querySelector<HTMLButtonElement>(
      'button[aria-label="Enlarge Fixture chart"]',
    ),
  )
  await act(async () => userEvent.click(imageTrigger))
  if (document.body.querySelector('.image-lightbox-dialog') === null) {
    throw new Error('Expected the question image lightbox')
  }

  await act(async () => userEvent.keyboard('{Escape}'))

  const getQuestionImageEscapeState = () => ({
    skipCalls: onSkip.mock.calls,
    lightboxOpen:
      document.body.querySelector('.image-lightbox-dialog') !== null,
    questionPresent: container.querySelector('.chat-question-card') !== null,
  })
  expect(getQuestionImageEscapeState()).toEqual({
    skipCalls: [],
    lightboxOpen: false,
    questionPresent: true,
  })
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
