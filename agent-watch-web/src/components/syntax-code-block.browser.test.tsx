import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SyntaxCodeBlock } from '#components/syntax-code-block'

vi.mock('./syntax-highlighter', () => ({
  highlightCode: () => Promise.resolve(null),
}))

let container: HTMLDivElement
let root: Root | null = null
let clipboardDescriptor: PropertyDescriptor | undefined
let execCommandDescriptor: PropertyDescriptor | undefined

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  clipboardDescriptor = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
  execCommandDescriptor = Object.getOwnPropertyDescriptor(
    document,
    'execCommand',
  )
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  if (clipboardDescriptor === undefined) {
    Reflect.deleteProperty(navigator, 'clipboard')
  } else {
    Object.defineProperty(navigator, 'clipboard', clipboardDescriptor)
  }
  clipboardDescriptor = undefined
  if (execCommandDescriptor === undefined) {
    Reflect.deleteProperty(document, 'execCommand')
  } else {
    Object.defineProperty(document, 'execCommand', execCommandDescriptor)
  }
  execCommandDescriptor = undefined
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

it('copies the full code and returns the button to its ready state after 1.5 seconds', async () => {
  const code = 'const preview = true\nconsole.log(preview)\n'
  const writeText = vi.fn<(text: string) => Promise<void>>(() =>
    Promise.resolve(),
  )
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })
  vi.useFakeTimers()
  root = createRoot(container)
  act(() => {
    root?.render(<SyntaxCodeBlock code={code} language="typescript" copyable />)
  })

  const button = container.querySelector<HTMLButtonElement>(
    '.syntax-code-block-copy-button',
  )
  if (button === null) throw new Error('Expected the code copy button')
  act(() => {
    button.click()
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
  const states = [
    {
      clipboardCalls: writeText.mock.calls,
      buttonText: button.textContent,
    },
  ]
  await act(async () => vi.advanceTimersByTimeAsync(1499))
  states.push({
    clipboardCalls: writeText.mock.calls,
    buttonText: button.textContent,
  })
  await act(async () => vi.advanceTimersByTimeAsync(1))
  states.push({
    clipboardCalls: writeText.mock.calls,
    buttonText: button.textContent,
  })

  expect(states).toEqual([
    { clipboardCalls: [[code]], buttonText: '✓ Copied' },
    { clipboardCalls: [[code]], buttonText: '✓ Copied' },
    { clipboardCalls: [[code]], buttonText: 'Copy' },
  ])
})

it('announces when the code cannot be copied', async () => {
  const code = 'const preview = true\n'
  const writeText = vi.fn<(text: string) => Promise<void>>(() =>
    Promise.reject(new Error('Copy denied')),
  )
  const execCommandCalls: string[] = []
  const execCommand = vi.fn((command: string) => {
    execCommandCalls.push(command)
    return false
  })
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: execCommand,
  })
  root = createRoot(container)
  act(() => {
    root?.render(<SyntaxCodeBlock code={code} language="typescript" copyable />)
  })

  const button = container.querySelector<HTMLButtonElement>(
    '.syntax-code-block-copy-button',
  )
  if (button === null) throw new Error('Expected the code copy button')
  act(() => {
    button.click()
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const getActual = () => ({
    clipboardCalls: writeText.mock.calls,
    execCommandCalls,
    buttonText: button.textContent,
    ariaLabel: button.getAttribute('aria-label'),
    announcement: container.querySelector('[role="status"]')?.textContent,
  })
  expect(getActual()).toEqual({
    clipboardCalls: [[code]],
    execCommandCalls: ['copy'],
    buttonText: 'Copy failed',
    ariaLabel: 'Could not copy code',
    announcement: 'Could not copy code',
  })
})
