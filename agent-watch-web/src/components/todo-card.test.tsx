import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import { TodoCard } from '#components/todo-card'
import type { SessionLinkTarget } from '#types'

const { optimizeImageAttachmentMock } = vi.hoisted(() => ({
  optimizeImageAttachmentMock: vi.fn<(file: File) => Promise<File>>(),
}))

vi.mock('#lib/image-attachment', () => ({
  optimizeImageAttachment: optimizeImageAttachmentMock,
}))

type Props = ComponentProps<typeof TodoCard>

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
    title: 'Check the updated layout on a phone',
    detail: 'Confirm the action stays visible on a small screen.',
    resolution: null,
    onResolve: vi.fn(() => Promise.resolve(true)),
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
        <TodoCard {...cardProps} />
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
    if (valueSetter !== undefined) Reflect.apply(valueSetter, textarea, [value])
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function normalizeBaseUiIds(markup: string | undefined) {
  return markup?.replace(/ id="base-ui-[^"]+"/g, ' id="[generated]"')
}

async function pasteImage(textarea: HTMLTextAreaElement, file: File) {
  const event = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'clipboardData', { value: { files: [file] } })
  await act(async () => {
    textarea.dispatchEvent(event)
    await new Promise((resolve) => setTimeout(resolve, 10))
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  onSelectSession.mockReset()
  optimizeImageAttachmentMock.mockReset()
  optimizeImageAttachmentMock.mockImplementation((file) =>
    Promise.resolve(file),
  )
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:todo-image')
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

it('shows a pending task with optional detail and a completion action', () => {
  renderCard(props())

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    title: container.querySelector('h3')?.textContent,
    detail: container.querySelector('.chat-todo-detail')?.textContent,
    commentLabel: container.querySelector('label')?.textContent,
    commentDisabled:
      container.querySelector<HTMLTextAreaElement>('.chat-todo-comment')
        ?.disabled,
    button: Array.from(
      container.querySelectorAll<HTMLButtonElement>('.chat-todo-footer button'),
      (button) => ({ text: button.textContent, disabled: button.disabled }),
    ),
  })

  expect(snapshot()).toEqual({
    status: 'To-do · pending',
    title: 'Check the updated layout on a phone',
    detail: 'Confirm the action stays visible on a small screen.',
    commentLabel: 'Comment (optional)',
    commentDisabled: false,
    button: [
      { text: 'Mark done', disabled: false },
      { text: 'Reject', disabled: false },
    ],
  })
})

it('does not offer to expand detail that fits in two lines', () => {
  renderCard(props({ detail: 'First line.\nSecond line.' }))

  const snapshot = () => ({
    detail: container.querySelector('.chat-todo-detail .markdown-message')
      ?.textContent,
    toggle: container.querySelector('.collapsible-text__toggle')?.textContent,
  })

  expect(snapshot()).toEqual({
    detail: 'First line.\nSecond line.',
    toggle: undefined,
  })
})

it('renders task detail and completion comments as markdown', () => {
  renderCard(
    props({
      detail: `Review ${linkedSessionId}, **the preview**, and [read the guide](https://example.test).`,
    }),
  )
  const detail = container.querySelector(
    '.chat-todo-detail .markdown-message',
  )?.innerHTML
  const detailLink = requireElement(
    container.querySelector<HTMLAnchorElement>('.chat-todo-detail a'),
  )
  act(() => {
    detailLink.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
  })
  renderCard(
    props({
      resolution: 'completed',
      defaultComment: `The ${linkedSessionId} [preview](https://example.test) is **ready**.`,
    }),
  )
  const completionLink = requireElement(
    container.querySelector<HTMLAnchorElement>(
      '.chat-todo-completed-comment a',
    ),
  )
  act(() => {
    completionLink.dispatchEvent(
      new MouseEvent('click', { bubbles: true, cancelable: true }),
    )
  })

  const snapshot = () => ({
    detail: normalizeBaseUiIds(detail),
    completionComment: normalizeBaseUiIds(
      container.querySelector('.chat-todo-completed-comment')?.innerHTML,
    ),
    selectedSessions: onSelectSession.mock.calls.map(([id]) => id),
  })

  expect(snapshot()).toEqual({
    detail: `<p>Review <a class="session-link-trigger" href="?session=${linkedSessionId}" rel="noopener noreferrer" target="_blank" id="[generated]">${linkedSessionId}</a>, <strong>the preview</strong>, and <a href="https://example.test" target="_blank" rel="noopener noreferrer">read the guide</a>.</p>`,
    completionComment: `<p>The <a class="session-link-trigger" href="?session=${linkedSessionId}" rel="noopener noreferrer" target="_blank" id="[generated]">${linkedSessionId}</a> <a href="https://example.test" target="_blank" rel="noopener noreferrer">preview</a> is <strong>ready</strong>.</p>`,
    selectedSessions: [linkedSessionId, linkedSessionId],
  })
})

it('shows a saved image in the completed comment', () => {
  renderCard(
    props({
      resolution: 'completed',
      defaultComment: '![Sample screenshot](/api/images/fixture-image.png)',
    }),
  )

  expect(
    container.querySelector('.chat-todo-completed-comment')?.innerHTML,
  ).toBe(
    '<p><button class="image-lightbox-trigger" type="button" aria-label="Enlarge Sample screenshot"><img class="chat-user-image" alt="Sample screenshot" loading="lazy" decoding="async" src="/api/images/fixture-image.png"></button></p>',
  )
})

it('expands and collapses long detail', async () => {
  container.style.width = '240px'
  renderCard(
    props({
      detail: Array.from(
        { length: 10 },
        () => 'Keep each section readable when the screen is narrow.',
      ).join(' '),
    }),
  )

  const detail = requireElement(
    container.querySelector<HTMLDivElement>(
      '.chat-todo-detail .collapsible-text__content',
    ),
  )
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>('.collapsible-text__toggle'),
  )
  const collapsedHeight = detail.getBoundingClientRect().height
  const firstToggle = {
    label: toggle.textContent,
    expanded: toggle.getAttribute('aria-expanded'),
  }
  await act(async () => {
    await userEvent.click(toggle)
  })
  const expandedHeight = detail.getBoundingClientRect().height
  const expandedToggle = {
    label: toggle.textContent,
    expanded: toggle.getAttribute('aria-expanded'),
  }
  await act(async () => {
    await userEvent.click(toggle)
  })

  const snapshot = () => ({
    collapsedHeightAfterClose: detail.getBoundingClientRect().height,
    expandedHeightGreaterThanCollapsed: expandedHeight > collapsedHeight,
    firstToggle,
    expandedToggle,
    finalToggle: {
      label: toggle.textContent,
      expanded: toggle.getAttribute('aria-expanded'),
    },
  })

  expect(snapshot()).toEqual({
    collapsedHeightAfterClose: collapsedHeight,
    expandedHeightGreaterThanCollapsed: true,
    firstToggle: {
      label: 'Show more',
      expanded: 'false',
    },
    expandedToggle: {
      label: 'Show less',
      expanded: 'true',
    },
    finalToggle: {
      label: 'Show more',
      expanded: 'false',
    },
  })
})

it('sends a trimmed optional comment and keeps the task as completed', async () => {
  const onResolve = vi.fn<Props['onResolve']>(() => Promise.resolve(true))
  renderCard(props({ onResolve }))
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  setTextareaValue(comment, '  The layout looks good.  ')
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.chat-todo-footer button'),
    ).click()
    await new Promise((resolve) => setTimeout(resolve, 100))
  })

  const snapshot = () => ({
    resolveCalls: onResolve.mock.calls,
    status: container.querySelector('h2')?.textContent,
    title: container.querySelector('h3')?.textContent,
    completionComment: container.querySelector('.chat-todo-completed-comment')
      ?.textContent,
    commentInput: container.querySelector('.chat-todo-comment') !== null,
    buttonCount: container.querySelectorAll('.chat-todo-footer button').length,
  })

  expect(snapshot()).toEqual({
    resolveCalls: [['completed', 'The layout looks good.']],
    status: 'To-do · done',
    title: 'Check the updated layout on a phone',
    completionComment: 'The layout looks good.',
    commentInput: false,
    buttonCount: 0,
  })
})

it('restores a task comment after reload and clears it after resolving', async () => {
  const sessionId = crypto.randomUUID()
  const seq = 42
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'todo',
    sessionId,
    seq,
    'comment',
  ])}`
  const onResolve = vi.fn<Props['onResolve']>(() => Promise.resolve(true))
  const cardProps = props({ sessionId, seq, onResolve })
  renderCard(cardProps)
  setTextareaValue(
    requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
    ),
    'The page works at narrow widths.',
  )
  const storedDraftBeforeReload = window.localStorage.getItem(storageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderCard(cardProps)

  const restoredComment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  const persistedBeforeSend = restoredComment.value
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.chat-todo-footer button'),
    ).click()
    await new Promise((resolve) => setTimeout(resolve, 100))
  })

  const snapshot = () => ({
    restoredComment: persistedBeforeSend,
    resolveCalls: onResolve.mock.calls,
    storedDraftBeforeReload,
    storedDraft: window.localStorage.getItem(storageKey),
  })

  expect(snapshot()).toEqual({
    restoredComment: 'The page works at narrow widths.',
    resolveCalls: [['completed', 'The page works at narrow widths.']],
    storedDraftBeforeReload: 'The page works at narrow widths.',
    storedDraft: null,
  })
})

it('keeps task comments after an unsuccessful resolution and reload', async () => {
  const sessionId = crypto.randomUUID()
  const seq = 43
  const storageKey = `agent-watch-web:card-draft:${JSON.stringify([
    'todo',
    sessionId,
    seq,
    'comment',
  ])}`
  const onResolve = vi.fn<Props['onResolve']>(() => Promise.resolve(false))
  const cardProps = props({ sessionId, seq, onResolve })
  renderCard(cardProps)
  setTextareaValue(
    requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
    ),
    'Retain the review note for another attempt.',
  )
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.chat-todo-footer button'),
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
    resolveCalls: onResolve.mock.calls,
    restoredComment: requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
    ).value,
    storedDraftAfterFailure,
    storedDraftAfterReload: window.localStorage.getItem(storageKey),
  })

  expect(snapshot()).toEqual({
    resolveCalls: [
      ['completed', 'Retain the review note for another attempt.'],
    ],
    restoredComment: 'Retain the review note for another attempt.',
    storedDraftAfterFailure: 'Retain the review note for another attempt.',
    storedDraftAfterReload: 'Retain the review note for another attempt.',
  })
})

it('previews and sends an image pasted into the task comment', async () => {
  let resolveTodo: (succeeded: boolean) => void = () => undefined
  const todoResult = new Promise<boolean>((resolve) => {
    resolveTodo = resolve
  })
  let notifyTodoCalled: () => void = () => undefined
  const todoCalled = new Promise<void>((resolve) => {
    notifyTodoCalled = resolve
  })
  const onResolve = vi.fn<Props['onResolve']>(() => {
    notifyTodoCalled()
    return todoResult
  })
  const sessionId = crypto.randomUUID()
  renderCard(props({ sessionId, seq: 38, onResolve }))
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  await pasteImage(
    comment,
    new File(['image'], 'screen.png', { type: 'image/png' }),
  )

  const previewBeforeSend = container
    .querySelector<HTMLImageElement>('.attachment-preview-item img')
    ?.getAttribute('src')
  const resolveButton = requireElement(
    container.querySelector<HTMLButtonElement>('.chat-todo-footer button'),
  )
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 100))
  })
  expect(resolveButton.disabled).toBe(false)
  await act(async () => {
    resolveButton.click()
    await todoCalled
  })
  const canRemoveWhileSending =
    container.querySelector('[aria-label="Remove screen.png"]') !== null
  await act(async () => {
    resolveTodo(true)
    await todoResult
  })

  const snapshot = () => ({
    previewBeforeSend,
    canRemoveWhileSending,
    resolveCalls: onResolve.mock.calls,
    sentAttachments: onResolve.mock.calls[0]?.[2],
    completedPreview: container
      .querySelector<HTMLImageElement>('.attachment-preview-item img')
      ?.getAttribute('src'),
  })
  expect(snapshot()).toEqual({
    previewBeforeSend: 'blob:todo-image',
    canRemoveWhileSending: false,
    resolveCalls: [
      [
        'completed',
        null,
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
    completedPreview: 'blob:todo-image',
  })
})

it('shows a completed task compactly with its saved comment', () => {
  renderCard(
    props({
      resolution: 'completed',
      defaultComment: 'The task was completed in another tab.',
    }),
  )

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    title: container.querySelector('h3')?.textContent,
    detail: container.querySelector('.chat-todo-detail')?.textContent,
    comment: container.querySelector('.chat-todo-completed-comment')
      ?.textContent,
    commentInput: container.querySelector('.chat-todo-comment') !== null,
    buttonCount: container.querySelectorAll('.chat-todo-footer button').length,
  })

  expect(snapshot()).toEqual({
    status: 'To-do · done',
    title: 'Check the updated layout on a phone',
    detail: undefined,
    comment: 'The task was completed in another tab.',
    commentInput: false,
    buttonCount: 0,
  })
})

it('shows the persisted comment when a pending task becomes completed', () => {
  renderCard(props())
  renderCard(
    props({
      resolution: 'completed',
      defaultComment: 'The task was completed in another tab.',
    }),
  )

  const snapshot = () => ({
    comment: container.querySelector('.chat-todo-completed-comment')
      ?.textContent,
    commentInput: container.querySelector('.chat-todo-comment') !== null,
    buttonCount: container.querySelectorAll('.chat-todo-footer button').length,
  })

  expect(snapshot()).toEqual({
    comment: 'The task was completed in another tab.',
    commentInput: false,
    buttonCount: 0,
  })
})

it('rejects a task with the trimmed optional comment', async () => {
  const onResolve = vi.fn(() => Promise.resolve(true))
  renderCard(props({ onResolve }))
  setTextareaValue(
    requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
    ),
    '  No longer needed.  ',
  )
  await act(async () => {
    const buttons = container.querySelectorAll<HTMLButtonElement>(
      '.chat-todo-footer button',
    )
    requireElement(buttons.item(1)).click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    calls: onResolve.mock.calls,
    status: container.querySelector('h2')?.textContent,
    resolution: container
      .querySelector('.chat-todo-card')
      ?.getAttribute('data-resolution'),
    comment: container.querySelector('.chat-todo-completed-comment')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    calls: [['rejected', 'No longer needed.']],
    status: 'To-do · rejected',
    resolution: 'rejected',
    comment: 'No longer needed.',
  })
})

it('shows a withdrawn task as distinct from a completed task', () => {
  renderCard(
    props({ resolution: 'withdrawn', defaultComment: 'The plan changed.' }),
  )

  const snapshot = () => ({
    status: container.querySelector('h2')?.textContent,
    resolution: container
      .querySelector('.chat-todo-card')
      ?.getAttribute('data-resolution'),
    comment: container.querySelector('.chat-todo-completed-comment')
      ?.textContent,
    buttonCount: container.querySelectorAll('.chat-todo-footer button').length,
  })
  expect(snapshot()).toEqual({
    status: 'To-do · withdrawn',
    resolution: 'withdrawn',
    comment: 'The plan changed.',
    buttonCount: 0,
  })
})
