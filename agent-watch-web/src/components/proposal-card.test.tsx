import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import { ProposalCard } from '#components/proposal-card'
import type { SessionLinkTarget } from '#types'

const { optimizeImageAttachmentMock } = vi.hoisted(() => ({
  optimizeImageAttachmentMock: vi.fn<(file: File) => Promise<File>>(),
}))

vi.mock('#lib/image-attachment', () => ({
  optimizeImageAttachment: optimizeImageAttachmentMock,
}))

type Props = ComponentProps<typeof ProposalCard>

const linkedSessionId = '11111111-1111-4111-8111-111111111111'
const linkedSession: SessionLinkTarget = {
  id: linkedSessionId,
  repo: 'example/toolkit',
  task: null,
  title: 'Inspect the retry path',
}
const onSelectSession = vi.fn<(id: string) => void>()

let container: HTMLDivElement
let root: Root | null = null

function props(overrides: Partial<Props> = {}): Props {
  return {
    sessionId: crypto.randomUUID(),
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
    root?.render(
      <MarkdownSessionLinksProvider
        sessions={[linkedSession]}
        onSelectSession={onSelectSession}
      >
        <ProposalCard {...cardProps} />
      </MarkdownSessionLinksProvider>,
    )
  })
}

function renderCards(cardProps: Props[]) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <MarkdownSessionLinksProvider
        sessions={[linkedSession]}
        onSelectSession={onSelectSession}
      >
        <>
          {cardProps.map((card) => (
            <ProposalCard
              {...card}
              key={`${card.sessionId ?? 'proposal'}-${String(card.seq)}`}
            />
          ))}
        </>
      </MarkdownSessionLinksProvider>,
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

async function pasteImage(textarea: HTMLTextAreaElement, file: File) {
  const event = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'clipboardData', { value: { files: [file] } })
  await act(async () => {
    textarea.dispatchEvent(event)
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}

async function dropImage(textarea: HTMLTextAreaElement, file: File) {
  const event = new Event('drop', { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'dataTransfer', {
    value: { types: ['Files'], files: [file], dropEffect: 'none' },
  })
  await act(async () => {
    textarea.dispatchEvent(event)
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
  return event
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
  onSelectSession.mockReset()
  optimizeImageAttachmentMock.mockReset()
  optimizeImageAttachmentMock.mockImplementation((file) =>
    Promise.resolve(file),
  )
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:proposal-image')
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

it('links session IDs in proposal summary, options, answers, and withdrawal comments', () => {
  renderCards([
    props({
      summary: `Compare the choices with ${linkedSessionId}.`,
      options: props().options.map((option, index) =>
        index === 0
          ? {
              ...option,
              description: `Follow the detail in ${linkedSessionId}.`,
            }
          : option,
      ),
      answer: { choice: 0, comment: `See the result in ${linkedSessionId}.` },
      resolved: true,
    }),
    props({
      seq: 8,
      resolution: 'withdrawn',
      resolved: true,
      withdrawalComment: `The session ${linkedSessionId} explains the change.`,
    }),
  ])

  const links = Array.from(
    container.querySelectorAll<HTMLAnchorElement>('.chat-proposal-card a'),
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
  })
  expect(getActual()).toEqual({
    links: Array.from({ length: 4 }, () => ({
      text: linkedSessionId,
      href: `?session=${linkedSessionId}`,
    })),
    selectedSessions: Array.from({ length: 4 }, () => linkedSessionId),
  })
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
        text: 'Use the existing flowRecommendedExplain the change before applying it.+ Small change− Adds one step',
        checked: false,
        recommended: 'Recommended',
      },
      {
        text: 'Show the choice earlierMove the choice before the next action.+ Clearer timing',
        checked: false,
        recommended: undefined,
      },
    ],
    buttons: [
      { text: 'Choose', disabled: true },
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

it('shows a saved image in an answered proposal comment', () => {
  renderCard(
    props({
      answer: {
        choice: 0,
        comment: '![Sample screenshot](/api/images/fixture-image.png)',
      },
      resolved: true,
    }),
  )

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    comment: container.querySelector('.chat-proposal-answer-comment')
      ?.innerHTML,
    commentInput: container.querySelector('.chat-proposal-comment') !== null,
    attachmentPreview:
      container.querySelector('.attachment-preview-list') !== null,
  })

  expect(snapshot()).toEqual({
    status: 'Proposal · chose Use the existing flow',
    comment:
      '<p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge Sample screenshot"><img class="chat-user-image" alt="Sample screenshot" loading="lazy" decoding="async" src="/api/images/fixture-image.png"></button></p>',
    commentInput: false,
    attachmentPreview: false,
  })
})

it('keeps option labels phrasing-only and links associated with their radios', () => {
  renderCard(
    props({
      sessionId: 'session-alpha',
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
    inputId: 'proposal-session-alpha-7-option-0',
    labelFor: 'proposal-session-alpha-7-option-0',
    labelText: 'Use the existing flow',
    labelLinkCount: 0,
    descriptionTag: 'DIV',
    descriptionLink: 'https://example.test',
  })
})

it('selects an option from its description but not from its links or copy button', () => {
  renderCard(
    props({
      options: [
        {
          label: 'Use the existing flow',
          description:
            'Read [the guide](https://example.test).\n\n~~~text\nsample\n~~~',
          pros: [],
          cons: [],
          recommended: false,
        },
        {
          label: 'Show the choice earlier',
          description: 'Move the choice before the next action.',
          pros: [],
          cons: [],
          recommended: false,
        },
      ],
    }),
  )

  const firstDescription = requireElement(
    container.querySelector<HTMLParagraphElement>(
      '.chat-proposal-option-description p',
    ),
  )
  const secondDescription = requireElement(
    container.querySelectorAll<HTMLParagraphElement>(
      '.chat-proposal-option-description p',
    )[1] ?? null,
  )
  const link = requireElement(
    container.querySelector<HTMLAnchorElement>(
      '.chat-proposal-option-description a',
    ),
  )
  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-proposal-option-description .syntax-code-block-copy-button',
    ),
  )
  link.addEventListener('click', (event) => {
    event.preventDefault()
  })

  const selected = () =>
    Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      ),
      (input) => input.checked,
    )

  act(() => {
    firstDescription.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
  })
  const afterFirstDescriptionClick = selected()
  act(() => {
    secondDescription.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
  })
  const afterSecondDescriptionClick = selected()
  act(() => {
    link.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
  })
  act(() => {
    copyButton.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
  })

  const snapshot = () => ({
    afterFirstDescriptionClick,
    afterSecondDescriptionClick,
    afterLinkClick: selected(),
    afterCopyClick: selected(),
  })

  expect(snapshot()).toEqual({
    afterFirstDescriptionClick: [true, false],
    afterSecondDescriptionClick: [false, true],
    afterLinkClick: [false, true],
    afterCopyClick: [false, true],
  })
})

it('keeps a proposal choice in its own session when labels share a sequence number', async () => {
  container.className = 'conversation-scroll'
  container.style.cssText = 'width: 420px; height: 260px; overflow-y: auto'
  renderCards([
    props({ sessionId: 'session-alpha' }),
    props({ sessionId: 'session-beta' }),
  ])
  const cards = Array.from(container.querySelectorAll('.chat-proposal-card'))
  const firstCard = cards[0]
  const secondCard = cards[1]
  if (firstCard === undefined || secondCard === undefined) {
    throw new Error('Expected two proposal cards')
  }
  const spacer = document.createElement('div')
  spacer.style.height = '800px'
  firstCard.after(spacer)
  const secondLabel = requireElement(
    secondCard.querySelector<HTMLLabelElement>('.chat-proposal-option-label'),
  )
  const scroll = container
  scroll.scrollTop +=
    secondLabel.getBoundingClientRect().top -
    scroll.getBoundingClientRect().top -
    40
  act(() => {
    scroll.dispatchEvent(new Event('scroll'))
  })
  const scrollBeforeSelection = scroll.scrollTop

  await act(async () => {
    await userEvent.click(secondLabel)
  })
  const scrollAfterClick = scroll.scrollTop
  const scrollBeforeKeyboard = scroll.scrollTop
  act(() => {
    requireElement(
      secondCard.querySelector<HTMLInputElement>('.chat-proposal-option input'),
    ).blur()
  })
  dispatchKey(window, '2')

  const snapshot = () => ({
    scrollBeforeSelection,
    scrollAfterClick,
    scrollBeforeKeyboard,
    scrollAfterKeyboard: scroll.scrollTop,
    selectedChoices: cards.map((card) =>
      Array.from(
        card.querySelectorAll<HTMLInputElement>('.chat-proposal-option input'),
        (input) => input.checked,
      ),
    ),
    commentLabelsTargetOwnTextarea: cards.map((card) => {
      const commentLabel = requireElement(
        card.querySelector<HTMLLabelElement>('.chat-proposal-comment-label'),
      )
      const comment = requireElement(
        card.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
      )
      return commentLabel.control === comment
    }),
  })
  expect(snapshot()).toEqual({
    scrollBeforeSelection,
    scrollAfterClick: scrollBeforeSelection,
    scrollBeforeKeyboard: scrollBeforeSelection,
    scrollAfterKeyboard: scrollBeforeSelection,
    selectedChoices: [
      [false, false],
      [false, true],
    ],
    commentLabelsTargetOwnTextarea: [true, true],
  })
})

it('selects a numbered option and sends it with the comment on Command-Enter', async () => {
  const onAnswer = vi.fn<Props['onAnswer']>(() => Promise.resolve(true))
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
    status: 'Proposal · chose Show the choice earlier',
    selected: [false, true],
    disabled: [true, true],
    comment: 'Keep the earlier prompt short.',
  })
})

it('restores a proposal comment after reload and clears it after sending', async () => {
  const sessionId = crypto.randomUUID()
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'proposal',
    sessionId,
    7,
    'comment',
  ])}`
  const choiceStorageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'proposal',
    sessionId,
    7,
    'choice',
  ])}`
  const onAnswer = vi.fn<Props['onAnswer']>(() => Promise.resolve(true))
  const cardProps = props({ sessionId, onAnswer })
  renderCard(cardProps)
  dispatchKey(window, '2')

  setTextareaValue(
    requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
    ),
    'Keep the new choice easy to compare.',
  )
  const storedDraftBeforeReload = window.localStorage.getItem(storageKey)
  const storedChoiceBeforeReload = window.localStorage.getItem(choiceStorageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderCard(cardProps)

  const restoredComment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const persistedBeforeSend = restoredComment.value
  const selectedAfterReload = Array.from(
    container.querySelectorAll<HTMLInputElement>('.chat-proposal-option input'),
    (input) => input.checked,
  )
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-proposal-actions button',
      ),
    ).click()
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    restoredComment: persistedBeforeSend,
    selectedAfterReload,
    answerCalls: onAnswer.mock.calls,
    storedDraftBeforeReload,
    storedChoiceBeforeReload,
    storedDraft: window.localStorage.getItem(storageKey),
    storedChoiceDraft: window.localStorage.getItem(choiceStorageKey),
  })

  expect(snapshot()).toEqual({
    restoredComment: 'Keep the new choice easy to compare.',
    selectedAfterReload: [false, true],
    answerCalls: [
      [{ choice: 1, comment: 'Keep the new choice easy to compare.' }],
    ],
    storedDraftBeforeReload: 'Keep the new choice easy to compare.',
    storedChoiceBeforeReload: '1',
    storedDraft: null,
    storedChoiceDraft: null,
  })
})

it('keeps proposal comments after an unsuccessful answer and reload', async () => {
  const sessionId = crypto.randomUUID()
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'proposal',
    sessionId,
    7,
    'comment',
  ])}`
  const onAnswer = vi.fn<Props['onAnswer']>(() => Promise.resolve(false))
  const cardProps = props({ sessionId, onAnswer })
  renderCard(cardProps)
  dispatchKey(window, '1')
  setTextareaValue(
    requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
    ),
    'Keep this note until the request succeeds.',
  )
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-proposal-actions button',
      ),
    ).click()
    await Promise.resolve()
  })
  const storedDraftAfterFailure = window.localStorage.getItem(storageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderCard(cardProps)

  const snapshot = () => ({
    answerCalls: onAnswer.mock.calls,
    restoredComment: requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
    ).value,
    storedDraftAfterFailure,
    storedDraftAfterReload: window.localStorage.getItem(storageKey),
  })

  expect(snapshot()).toEqual({
    answerCalls: [
      [{ choice: 0, comment: 'Keep this note until the request succeeds.' }],
    ],
    restoredComment: 'Keep this note until the request succeeds.',
    storedDraftAfterFailure: 'Keep this note until the request succeeds.',
    storedDraftAfterReload: 'Keep this note until the request succeeds.',
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
    statuses: [
      'Proposal · waiting for your choice',
      'Proposal · chose Show the choice earlier',
    ],
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

it('previews and sends an image pasted into the proposal comment', async () => {
  let resolveAnswer: (succeeded: boolean) => void = () => undefined
  const answerResult = new Promise<boolean>((resolve) => {
    resolveAnswer = resolve
  })
  let notifyAnswerCalled: () => void = () => undefined
  const answerCalled = new Promise<void>((resolve) => {
    notifyAnswerCalled = resolve
  })
  const onAnswer = vi.fn<Props['onAnswer']>(() => {
    notifyAnswerCalled()
    return answerResult
  })
  const sessionId = crypto.randomUUID()
  renderCard(props({ sessionId, defaultChoice: 0, onAnswer }))
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  await pasteImage(
    comment,
    new File(['image'], 'screen.png', { type: 'image/png' }),
  )

  const previewBeforeSend = container
    .querySelector<HTMLImageElement>('.attachment-preview-item img')
    ?.getAttribute('src')
  const chooseButton = requireElement(
    container.querySelector<HTMLButtonElement>('.chat-proposal-actions button'),
  )
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 100))
  })
  expect(chooseButton.disabled).toBe(false)
  await act(async () => {
    chooseButton.click()
    await answerCalled
  })
  const canRemoveWhileSending =
    container.querySelector('[aria-label="Remove screen.png"]') !== null
  await act(async () => {
    resolveAnswer(true)
    await answerResult
  })

  const snapshot = () => ({
    previewBeforeSend,
    canRemoveWhileSending,
    answerCalls: onAnswer.mock.calls,
    sentAttachments: onAnswer.mock.calls[0]?.[1],
    completedPreview: container
      .querySelector<HTMLImageElement>('.attachment-preview-item img')
      ?.getAttribute('src'),
  })
  expect(snapshot()).toEqual({
    previewBeforeSend: 'blob:proposal-image',
    canRemoveWhileSending: false,
    answerCalls: [
      [
        { choice: 0, comment: null },
        [
          {
            name: 'screen.png',
            mimeType: 'image/png',
            size: 5,
            data: 'aW1hZ2U=',
          },
        ],
      ],
    ],
    sentAttachments: [
      {
        name: 'screen.png',
        mimeType: 'image/png',
        size: 5,
        data: 'aW1hZ2U=',
      },
    ],
    completedPreview: 'blob:proposal-image',
  })
})

it('attaches dropped images to the proposal comment without bubbling to the conversation', async () => {
  renderCard(props())
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const conversationDrop = vi.fn()
  document.body.addEventListener('drop', conversationDrop)
  const drop = await dropImage(
    comment,
    new File(['image'], 'dropped.png', { type: 'image/png' }),
  )
  document.body.removeEventListener('drop', conversationDrop)

  const snapshot = () => ({
    dropPrevented: drop.defaultPrevented,
    conversationDropCalls: conversationDrop.mock.calls,
    attachmentNames: Array.from(
      container.querySelectorAll('.attachment-preview-item span'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    dropPrevented: true,
    conversationDropCalls: [],
    attachmentNames: ['dropped.png'],
  })
})

it('lets the conversation handle drops over a withdrawn proposal comment', async () => {
  renderCard(
    props({
      resolved: true,
      resolution: 'withdrawn',
      withdrawalComment: 'This proposal is no longer active.',
    }),
  )
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const conversationDrops: string[] = []
  const onConversationDrop = () => conversationDrops.push('drop')
  document.body.addEventListener('drop', onConversationDrop)
  const drop = await dropImage(
    comment,
    new File(['image'], 'dropped.png', { type: 'image/png' }),
  )
  document.body.removeEventListener('drop', onConversationDrop)

  const snapshot = () => ({
    dropPrevented: drop.defaultPrevented,
    conversationDrops,
  })
  expect(snapshot()).toEqual({
    dropPrevented: false,
    conversationDrops: ['drop'],
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
