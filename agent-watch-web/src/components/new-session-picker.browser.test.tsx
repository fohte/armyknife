import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { NewSessionPicker } from '#components/new-session-picker'
import {
  exampleModelOptions,
  exampleSessionDirectories,
} from '#new-session-story-fixtures.test'
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
  chooseModel = () => undefined,
  setSelectedEffort = () => undefined,
  selectedDirectory: initialSelectedDirectory = null,
  selectedModel = null,
  selectedEffort = null,
  activeModelOption = exampleModelOptions.models[0],
  modelOptionsError = null,
  initialModelPickerOpen = false,
}: {
  start: NewSessionState['start']
  onClose?: () => void
  chooseModel?: NewSessionState['chooseModel']
  setSelectedEffort?: NewSessionState['setSelectedEffort']
  selectedDirectory?: NewSessionState['selectedDirectory']
  selectedModel?: NewSessionState['selectedModel']
  selectedEffort?: NewSessionState['selectedEffort']
  activeModelOption?: NewSessionState['activeModelOption']
  modelOptionsError?: NewSessionState['modelOptionsError']
  initialModelPickerOpen?: boolean
}) {
  const [selectedDirectory, setSelectedDirectory] = useState<
    NewSessionState['selectedDirectory']
  >(initialSelectedDirectory)
  const [engine, setEngine] = useState<NewSessionState['engine']>('claude')
  const [worktree, setWorktree] = useState(false)
  const state: NewSessionState = {
    directoryQuery: '',
    directories: exampleSessionDirectories,
    selectedDirectory,
    engine,
    modelOptions: exampleModelOptions,
    isLoadingModelOptions: false,
    modelOptionsError,
    selectedModel,
    selectedEffort,
    activeModelOption,
    worktree,
    initialMessage: '',
    skillSuggestions: [],
    isLoadingDirectories: false,
    isStarting: false,
    error: null,
    setDirectoryQuery: () => undefined,
    chooseDirectory: setSelectedDirectory,
    setEngine,
    chooseModel,
    setSelectedEffort,
    setWorktree,
    setInitialMessage: () => undefined,
    start,
    reset: () => undefined,
  }
  return (
    <NewSessionPicker
      state={state}
      initialModelPickerOpen={initialModelPickerOpen}
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

it('selects the next directory with ArrowDown and Enter', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(<PickerHarness start={start} />)
  })
  await act(async () => {
    await userEvent.keyboard('{ArrowDown}{Enter}')
  })

  const snapshot = () => ({
    directory: container
      .querySelector('.new-session-picker-directory')
      ?.getAttribute('aria-label'),
    startCalls: start.mock.calls,
  })
  expect(snapshot()).toEqual({
    directory: 'Change directory, currently /Users/example/work/handbook',
    startCalls: [],
  })
})

it.each([
  ['composition state', true, undefined],
  ['legacy key code', false, 229],
] as const)(
  'does not launch during IME confirmation reported by %s',
  async (_signal, isComposing, keyCode) => {
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
    const event = new KeyboardEvent('keydown', {
      bubbles: true,
      isComposing,
      key: 'Enter',
    })
    if (keyCode !== undefined) {
      Object.defineProperty(event, 'keyCode', { value: keyCode })
    }
    act(() => {
      firstMessage.dispatchEvent(event)
    })

    expect(start.mock.calls).toEqual([])
  },
)

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

it('launches Codex when Option+Enter is pressed on the Claude Code option', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(
      <PickerHarness
        selectedDirectory={exampleSessionDirectories[0] ?? null}
        start={start}
      />,
    )
  })
  const claudeOption = requireButton(
    '.new-session-picker-options > button:first-of-type',
  )
  await act(async () => {
    claudeOption.focus()
    await userEvent.keyboard('{Alt>}{Enter}{/Alt}')
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

it('changes the model and effort with arrow keys in the model picker', async () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  const chooseModel = vi.fn<NewSessionState['chooseModel']>()
  const setSelectedEffort = vi.fn<NewSessionState['setSelectedEffort']>()
  root = createRoot(container)
  act(() => {
    root?.render(
      <PickerHarness
        chooseModel={chooseModel}
        setSelectedEffort={setSelectedEffort}
        start={start}
      />,
    )
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })
  await act(async () => {
    await userEvent.click(requireButton('.new-session-picker-model'))
  })
  await act(async () => {
    await userEvent.keyboard('{ArrowDown}{ArrowRight}')
  })

  const snapshot = () => ({
    modelCalls: chooseModel.mock.calls,
    effortCalls: setSelectedEffort.mock.calls,
  })
  expect(snapshot()).toEqual({
    modelCalls: [['fixture-model-orbit']],
    effortCalls: [['low']],
  })
})

it('shows the profile model in Default while another model is selected', () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(
      <PickerHarness
        activeModelOption={exampleModelOptions.models[1]}
        initialModelPickerOpen
        selectedDirectory={exampleSessionDirectories[0] ?? null}
        selectedEffort="high"
        selectedModel="fixture-model-comet"
        start={start}
      />,
    )
  })

  expect(
    container.querySelector(
      '.new-session-model-option .new-session-model-detail',
    )?.textContent,
  ).toEqual('fixture-model-orbit')
})

it('shows a catalog error alongside cached model and effort choices', () => {
  const start = vi.fn<NewSessionState['start']>(() => Promise.resolve())
  root = createRoot(container)
  act(() => {
    root?.render(
      <PickerHarness
        initialModelPickerOpen
        modelOptionsError="Model catalog refresh failed."
        selectedDirectory={exampleSessionDirectories[0] ?? null}
        selectedEffort="high"
        selectedModel="fixture-model-orbit"
        start={start}
      />,
    )
  })

  const snapshot = () => ({
    alert: container.querySelector('[role="alert"]')?.textContent,
    checkedModel: container.querySelector(
      '[aria-label="Model"] [aria-checked="true"]',
    )?.textContent,
    checkedEffort: container.querySelector(
      '[aria-label="Reasoning effort"] [aria-checked="true"]',
    )?.textContent,
  })
  expect(snapshot()).toEqual({
    alert: 'Model catalog refresh failed.',
    checkedModel: 'Orbitup to high',
    checkedEffort: 'high',
  })
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
