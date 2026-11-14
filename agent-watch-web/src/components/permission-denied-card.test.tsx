import '#index.css'

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
  inputSummary: 'A shell operation was denied.',
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

  const snapshot = () => {
    const card = container.querySelector<HTMLElement>('article')
    const heading = container.querySelector<HTMLElement>('h2')
    const input = container.querySelector<HTMLElement>(
      '.permission-denied-input',
    )
    const cardStyle = card === null ? null : getComputedStyle(card)
    const headingStyle = heading === null ? null : getComputedStyle(heading)
    const inputStyle = input === null ? null : getComputedStyle(input)
    return {
      cardClassName: card?.className,
      cardBorderColor: cardStyle?.borderTopColor ?? null,
      cardBorderRadius: cardStyle?.borderTopLeftRadius ?? null,
      heading: heading?.textContent,
      headingColor: headingStyle?.color ?? null,
      tool: container.querySelector('.permission-tool')?.textContent,
      input: input?.textContent,
      inputStyles:
        inputStyle === null
          ? null
          : {
              backgroundColor: inputStyle.backgroundColor,
              whiteSpace: inputStyle.whiteSpace,
              overflowWrap: inputStyle.overflowWrap,
            },
      reason: container.querySelector('.permission-denied-reason')?.textContent,
      button: Array.from(container.querySelectorAll('button'), (button) => ({
        text: button.textContent,
        disabled: button.disabled,
      })),
    }
  }

  expect(snapshot()).toEqual({
    cardClassName: 'chat-permission-card chat-permission-denied-card',
    cardBorderColor: 'rgb(251, 191, 36)',
    cardBorderRadius: '0px',
    heading: 'Operation denied',
    headingColor: 'rgb(251, 191, 36)',
    tool: 'Bash',
    input: 'A shell operation was denied.',
    inputStyles: {
      backgroundColor: 'rgb(19, 19, 19)',
      whiteSpace: 'pre-wrap',
      overflowWrap: 'anywhere',
    },
    reason: '[Sensitive Operation]',
    button: [{ text: 'Allow and retry', disabled: false }],
  })
})

it('highlights a denied Bash command while preserving its line breaks', async () => {
  const command = `printf '%s\\n' 'Checking preview'
git status --short`
  renderCard({
    message: {
      ...operation,
      inputSummary: 'A shell command was denied.',
      command,
    },
  })

  const snapshot = () => {
    const code = container.querySelector('.permission-denied-input code')
    return {
      code: code?.textContent ?? null,
      hasColoredToken:
        code !== null && code.querySelector('span[style]') !== null,
    }
  }

  await act(async () => {
    const { highlightCode } = await import('./syntax-highlighter')
    await highlightCode(command, 'bash')
  })

  expect(snapshot()).toEqual({ code: command, hasColoredToken: true })
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
