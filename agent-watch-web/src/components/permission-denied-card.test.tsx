import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { PermissionDeniedCard } from '#components/permission-denied-card'
import type { ChatItem } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const operation: Extract<ChatItem, { type: 'permission_denied' }> = {
  type: 'permission_denied',
  seq: 1,
  toolUseId: 'tool-call-example',
  tool: 'Bash',
  inputSummary: 'cargo test --workspace',
  reason: '[Sensitive Operation]',
  retryable: true,
  resolved: false,
}

function renderCard(
  overrides: Partial<Parameters<typeof PermissionDeniedCard>[0]> = {},
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <PermissionDeniedCard
        message={operation}
        canRetry
        actionsReady
        pendingAction={null}
        onRetry={() => undefined}
        {...overrides}
      />,
    )
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

it('renders the denied operation and offers a one-time retry', () => {
  renderCard()

  const snapshot = () => ({
    heading: container.querySelector('h2')?.textContent,
    tool: container.querySelector('.permission-denied-tool')?.textContent,
    input: container.querySelector('.permission-denied-input')?.textContent,
    reason: container.querySelector('.permission-denied-reason')?.textContent,
    button: Array.from(container.querySelectorAll('button'), (button) => ({
      text: button.textContent,
      disabled: button.disabled,
    })),
  })

  expect(snapshot()).toEqual({
    heading: 'Operation denied',
    tool: 'Bash',
    input: 'cargo test --workspace',
    reason: '[Sensitive Operation]',
    button: [{ text: 'Allow and retry', disabled: false }],
  })
})

it('sends no retry action for denials without a classifier verdict', () => {
  renderCard({
    message: {
      ...operation,
      reason:
        'Auto mode could not evaluate this action and is blocking it for safety.',
      retryable: false,
    },
  })

  const snapshot = () => ({
    buttonCount: container.querySelectorAll('button').length,
    unavailable: container.querySelector('.permission-denied-unavailable')
      ?.textContent,
  })

  expect(snapshot()).toEqual({
    buttonCount: 0,
    unavailable:
      'This denial has no classifier verdict and cannot be approved here.',
  })
})

it('calls the retry action once when the button is pressed', () => {
  const onRetry = vi.fn()
  renderCard({ onRetry })
  act(() => {
    requireElement(container.querySelector('button')).click()
  })

  const snapshot = () => ({
    calls: onRetry.mock.calls,
    sendingLabel: container.querySelector('button')?.textContent,
  })

  expect(snapshot()).toEqual({ calls: [[]], sendingLabel: 'Allow and retry' })
})
