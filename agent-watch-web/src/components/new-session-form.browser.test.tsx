import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { NewSessionForm } from '#components/new-session-form'
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

function makeState(overrides: Partial<NewSessionState> = {}): NewSessionState {
  return {
    directoryQuery: '',
    directories: exampleSessionDirectories,
    selectedDirectory: null,
    engine: 'claude',
    modelOptions: exampleModelOptions,
    isLoadingModelOptions: false,
    modelOptionsError: null,
    selectedModel: null,
    selectedEffort: null,
    modelChangeNotice: null,
    activeModelOption: exampleModelOptions.models[0],
    worktree: false,
    initialMessage: '',
    skillSuggestions: [],
    isLoadingDirectories: false,
    isStarting: false,
    error: null,
    setDirectoryQuery: () => undefined,
    chooseDirectory: () => undefined,
    setEngine: () => undefined,
    chooseModel: () => undefined,
    setSelectedEffort: () => undefined,
    undoEngineChange: () => undefined,
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

function StatefulSkillForm() {
  const [initialMessage, setInitialMessage] = useState('/')
  const state = makeState({
    initialMessage,
    skillSuggestions: [
      { name: 'design-flow', description: 'Help shape a design.' },
      { name: 'debug-flow', description: 'Investigate a failure.' },
    ],
    setInitialMessage,
  })
  return (
    <NewSessionForm
      state={state}
      onClose={() => undefined}
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

it('requires a directory before the session can start', () => {
  renderForm(makeState())

  expect(
    container.querySelector<HTMLButtonElement>('.new-session-start')?.disabled,
  ).toBe(true)
})

it('suggests and inserts skills in the first message field', async () => {
  root = createRoot(container)
  act(() => root?.render(<StatefulSkillForm />))

  const textarea = container.querySelector<HTMLTextAreaElement>(
    '[aria-label="First message"]',
  )
  if (textarea === null) throw new Error('Expected first message field')
  await act(async () => {
    await userEvent.click(textarea)
    await userEvent.keyboard('{ArrowDown}{Enter}')
  })

  const snapshot = () => ({
    draft: textarea.value,
    optionCount: container.querySelectorAll('[role="option"]').length,
    expanded: textarea.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    draft: '/debug-flow ',
    optionCount: 0,
    expanded: 'false',
  })
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

it('closes new session popovers after a click outside each control', () => {
  renderForm(makeState())
  const outside = container.querySelector<HTMLTextAreaElement>(
    '[aria-label="First message"]',
  )
  if (outside === null) throw new Error('Expected first message field')

  act(() => {
    requireButton('.new-session-directory-button').click()
  })
  act(() => {
    outside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })
  const directoryClosed = requireButton(
    '.new-session-directory-button',
  ).getAttribute('aria-expanded')

  act(() => {
    requireButton('.new-session-engine-control button').click()
  })
  act(() => {
    outside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })
  const engineClosed = container
    .querySelector('[aria-haspopup="menu"]')
    ?.getAttribute('aria-expanded')

  act(() => {
    requireButton('.new-session-model-trigger').click()
  })
  act(() => {
    outside.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })

  const snapshot = () => ({
    directoryClosed,
    engineClosed,
    modelClosed: requireButton('.new-session-model-trigger').getAttribute(
      'aria-expanded',
    ),
  })
  expect(snapshot()).toEqual({
    directoryClosed: 'false',
    engineClosed: 'false',
    modelClosed: 'false',
  })
})

it('closes skill suggestions after a click outside the composer', async () => {
  root = createRoot(container)
  act(() => root?.render(<StatefulSkillForm />))
  const heading = container.querySelector('h1')
  if (heading === null) throw new Error('Expected new session heading')

  await act(async () => {
    await userEvent.click(heading)
  })

  const snapshot = () => ({
    optionCount: container.querySelectorAll('[role="option"]').length,
    expanded: container
      .querySelector('[aria-label="First message"]')
      ?.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({ optionCount: 0, expanded: 'false' })
})

it('selects a model and supported effort from the model chip', async () => {
  const chooseModel = vi.fn()
  const setSelectedEffort = vi.fn()
  renderForm(
    makeState({
      chooseModel,
      setSelectedEffort,
    }),
  )
  await act(async () => {
    await userEvent.click(
      requireButton('[aria-label="Model and reasoning effort"]'),
    )
  })
  const modelOption = Array.from(
    container.querySelectorAll<HTMLButtonElement>('.new-session-model-option'),
  ).find((option) => option.textContent.includes('Comet'))
  if (modelOption === undefined) throw new Error('Expected the Comet model')
  const effortOption = Array.from(
    container.querySelectorAll<HTMLButtonElement>('.new-session-effort-option'),
  ).find((option) => option.textContent === 'high')
  if (effortOption === undefined) throw new Error('Expected high effort')

  await act(async () => {
    await userEvent.click(modelOption)
    await userEvent.click(effortOption)
  })

  const snapshot = () => ({
    modelCalls: chooseModel.mock.calls,
    effortCalls: setSelectedEffort.mock.calls,
  })
  expect(snapshot()).toEqual({
    modelCalls: [['fixture-model-comet']],
    effortCalls: [['high']],
  })
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
