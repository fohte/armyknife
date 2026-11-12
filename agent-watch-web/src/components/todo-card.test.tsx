import type { ComponentProps } from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { TodoCard } from '#components/todo-card'

type Props = ComponentProps<typeof TodoCard>

let container: HTMLDivElement
let root: Root | null = null

function props(overrides: Partial<Props> = {}): Props {
  return {
    title: 'Check the updated layout on a phone',
    detail: 'Confirm the action stays visible on a small screen.',
    completed: false,
    onComplete: vi.fn(() => Promise.resolve(true)),
    ...overrides,
  }
}

function renderCard(cardProps: Props) {
  root ??= createRoot(container)
  act(() => {
    root?.render(<TodoCard {...cardProps} />)
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
    button: [{ text: 'Mark done', disabled: false }],
  })
})

it('sends a trimmed optional comment and keeps the task as completed', async () => {
  const onComplete = vi.fn(() => Promise.resolve(true))
  renderCard(props({ onComplete }))
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  setTextareaValue(comment, '  The layout looks good.  ')
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.chat-todo-footer button'),
    ).click()
    await Promise.resolve()
  })

  const snapshot = () => ({
    completeCalls: onComplete.mock.calls,
    status: container.querySelector('h2')?.textContent,
    title: container.querySelector('h3')?.textContent,
    completionComment: container.querySelector('.chat-todo-completed-comment')
      ?.textContent,
    commentInput: container.querySelector('.chat-todo-comment') !== null,
    buttonCount: container.querySelectorAll('.chat-todo-footer button').length,
  })

  expect(snapshot()).toEqual({
    completeCalls: [['The layout looks good.']],
    status: 'To-do · done',
    title: 'Check the updated layout on a phone',
    completionComment: 'The layout looks good.',
    commentInput: false,
    buttonCount: 0,
  })
})

it('shows a completed task compactly with its saved comment', () => {
  renderCard(
    props({
      completed: true,
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
      completed: true,
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
