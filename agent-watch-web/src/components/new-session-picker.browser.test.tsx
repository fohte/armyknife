import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { NewSessionPicker } from '#components/new-session-picker'
import { exampleSessionDirectories } from '#new-session-story-fixtures.test'
import type { NewSessionState } from '#use-new-session'

let container: HTMLDivElement
let root: Root | null = null

function requireButton(selector: string) {
  const button = container.querySelector<HTMLButtonElement>(selector)
  if (button === null) throw new Error(`Expected button for ${selector}`)
  return button
}

function PickerHarness({
  start,
  onClose = () => undefined,
}: {
  start: NewSessionState['start']
  onClose?: () => void
}) {
  const [selectedDirectory, setSelectedDirectory] =
    useState<NewSessionState['selectedDirectory']>(null)
  const [engine, setEngine] = useState<NewSessionState['engine']>('claude')
  const [worktree, setWorktree] = useState(false)
  const state: NewSessionState = {
    directoryQuery: '',
    directories: exampleSessionDirectories,
    selectedDirectory,
    engine,
    worktree,
    initialMessage: '',
    isLoadingDirectories: false,
    isStarting: false,
    error: null,
    setDirectoryQuery: () => undefined,
    chooseDirectory: setSelectedDirectory,
    setEngine,
    setWorktree,
    setInitialMessage: () => undefined,
    start,
    reset: () => undefined,
  }
  return (
    <NewSessionPicker
      state={state}
      onClose={onClose}
      onLaunched={() => undefined}
    />
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

it('launches Codex when Option+Enter is pressed after choosing a directory', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness start={start} />)
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })
  await act(async () => {
    await userEvent.keyboard('{Alt>}{Enter}{/Alt}')
  })

  const snapshot = () =>
    start.mock.calls.map(([onLaunched, engine]) => ({
      callbackType: typeof onLaunched,
      engine,
    }))
  expect(snapshot()).toEqual([{ callbackType: 'function', engine: 'codex' }])
})

it('does not launch while the first message is being composed with an IME', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness start={start} />)
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })

  const firstMessage = container.querySelector<HTMLInputElement>(
    '[aria-label="First message"]',
  )
  if (firstMessage === null)
    throw new Error('First message input was not rendered')
  act(() => {
    firstMessage.dispatchEvent(
      new KeyboardEvent('keydown', {
        bubbles: true,
        isComposing: true,
        key: 'Enter',
      }),
    )
  })

  expect(start.mock.calls).toEqual([])
})

it('starts the selected engine from the desktop picker button', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness start={start} />)
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })
  await act(async () => {
    await userEvent.click(
      requireButton('[aria-label="Start session with Claude Code"]'),
    )
  })

  const snapshot = () =>
    start.mock.calls.map(([onLaunched, engine]) => [typeof onLaunched, engine])
  expect(snapshot()).toEqual([['function', 'claude']])
})

it('launches the focused engine option when Enter is pressed', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness start={start} />)
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })
  const codexOption = requireButton('.new-session-picker-option:nth-of-type(2)')
  await act(async () => {
    await userEvent.click(codexOption)
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () =>
    start.mock.calls.map(([onLaunched, engine]) => [typeof onLaunched, engine])
  expect(snapshot()).toEqual([['function', 'codex']])
})

it('toggles the worktree option with Option+W', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness start={start} />)
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })
  await act(async () => {
    await userEvent.keyboard('{Alt>}w{/Alt}')
  })

  expect(
    container
      .querySelector<HTMLButtonElement>('.new-session-picker-worktree')
      ?.getAttribute('aria-pressed'),
  ).toBe('true')
})

it('closes the picker when Enter activates the close button', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  const onClose = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness onClose={onClose} start={start} />)
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })
  const closeButton = requireButton('.new-session-picker-close')
  act(() => {
    closeButton.focus()
  })
  await act(async () => {
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    closeCalls: onClose.mock.calls,
    startCalls: start.mock.calls.map(([, engine]) => engine),
  })
  expect(snapshot()).toEqual({ closeCalls: [[]], startCalls: [] })
})
