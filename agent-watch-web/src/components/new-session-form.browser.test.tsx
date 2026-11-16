import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { NewSessionForm } from '#components/new-session-form'
import { exampleSessionDirectories } from '#new-session-story-fixtures'
import type { NewSessionState } from '#use-new-session'

let container: HTMLDivElement
let root: Root | null = null

function requireButton(selector: string) {
  const button = container.querySelector<HTMLButtonElement>(selector)
  if (button === null) throw new Error(`Expected button for ${selector}`)
  return button
}

function makeState(overrides: Partial<NewSessionState> = {}): NewSessionState {
  return {
    directoryQuery: '',
    directories: exampleSessionDirectories,
    selectedDirectory: null,
    engine: 'claude',
    worktree: false,
    initialMessage: '',
    isLoadingDirectories: false,
    isStarting: false,
    error: null,
    setDirectoryQuery: () => undefined,
    chooseDirectory: () => undefined,
    setEngine: () => undefined,
    setWorktree: () => undefined,
    setInitialMessage: () => undefined,
    start: () => Promise.resolve(),
    reset: () => undefined,
    ...overrides,
  }
}

function renderForm(state: NewSessionState, onLaunched = vi.fn()) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <NewSessionForm
        state={state}
        onClose={() => undefined}
        onLaunched={onLaunched}
      />,
    )
  })
  return { onLaunched }
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

it('requires a directory before the session can start', () => {
  renderForm(makeState())

  expect(
    container.querySelector<HTMLButtonElement>('.new-session-start')?.disabled,
  ).toBe(true)
})

it('opens directory suggestions and selects the clicked directory', async () => {
  const chooseDirectory = vi.fn()
  renderForm(makeState({ chooseDirectory }))
  await act(async () => {
    await userEvent.click(requireButton('.new-session-directory-button'))
  })
  await act(async () => {
    await userEvent.click(
      requireButton(
        '[aria-label="Select directory /Users/example/work/service"]',
      ),
    )
  })

  expect(chooseDirectory.mock.calls).toEqual([[exampleSessionDirectories[0]]])
})

it('starts the session and reports the returned launch ID', async () => {
  const onLaunched = vi.fn()
  const start = vi.fn((callback: (launchId: string) => void) => {
    callback('launch-fixture')
    return Promise.resolve()
  })
  renderForm(
    makeState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      start,
    }),
    onLaunched,
  )
  await act(async () => {
    await userEvent.click(requireButton('.new-session-start'))
  })

  const snapshot = () => ({
    startCalls: start.mock.calls.length,
    launched: onLaunched.mock.calls,
  })
  expect(snapshot()).toEqual({
    startCalls: 1,
    launched: [['launch-fixture']],
  })
})
