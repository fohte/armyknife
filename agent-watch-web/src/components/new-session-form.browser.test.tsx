import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { NewSessionForm } from '#components/new-session-form'
import type { LaunchSessionRequest } from '#new-session-api'
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
    activeModelOption: exampleModelOptions.models[0],
    worktree: false,
    initialMessage: '',
    restoredAt: null,
    skillSuggestions: [],
    isLoadingDirectories: false,
    error: null,
    setLaunchError: () => undefined,
    setDirectoryQuery: () => undefined,
    chooseDirectory: () => undefined,
    setEngine: () => undefined,
    chooseModel: () => undefined,
    setSelectedEffort: () => undefined,
    setWorktree: () => undefined,
    setInitialMessage: () => undefined,
    start: () => undefined,
    clearDraft: () => undefined,
    reset: () => undefined,
    ...overrides,
  }
}

function renderForm(
  state: NewSessionState,
  onStart = vi.fn(),
  launchPending = false,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <NewSessionForm
        state={state}
        launchPending={launchPending}
        onClose={() => undefined}
        onStart={onStart}
      />,
    )
  })
  return { onStart }
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
      onStart={() => undefined}
    />
  )
}

function StatefulKeyboardForm({
  start,
  isMobile = false,
}: {
  start: NewSessionState['start']
  isMobile?: boolean
}) {
  const [initialMessage, setInitialMessage] = useState('')
  return (
    <NewSessionForm
      state={makeState({
        initialMessage,
        selectedDirectory: exampleSessionDirectories[0] ?? null,
        setInitialMessage,
        start,
      })}
      isMobile={isMobile}
      onClose={() => undefined}
      onStart={() => undefined}
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

it('clears the restored draft from the form', async () => {
  const clearDraft = vi.fn()
  renderForm(
    makeState({
      restoredAt: '2026-05-06T12:03:00Z',
      clearDraft,
    }),
  )
  const clearButton = container.querySelector<HTMLButtonElement>(
    '.new-session-restored-notice button',
  )
  if (clearButton === null) throw new Error('Expected the Clear button')
  await act(async () => {
    await userEvent.click(clearButton)
  })

  expect(clearDraft.mock.calls).toEqual([[]])
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

it('uses the reply keyboard behavior for the first message', async () => {
  const start = vi.fn<NewSessionState['start']>()
  root = createRoot(container)
  act(() => root?.render(<StatefulKeyboardForm start={start} />))
  const textarea = container.querySelector<HTMLTextAreaElement>(
    '[aria-label="First message"]',
  )
  if (textarea === null) throw new Error('First message field was not rendered')
  await act(async () => {
    textarea.focus()
    await userEvent.keyboard('{Shift>}{Enter}{/Shift}{Alt>}{Enter}{/Alt}')
  })
  const afterNewlines = {
    message: textarea.value,
    starts: start.mock.calls.length,
  }
  await act(async () => {
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    afterNewlines,
    starts: start.mock.calls.map(([onStart]) => typeof onStart),
  })
  expect(snapshot()).toEqual({
    afterNewlines: { message: '\n\n', starts: 0 },
    starts: ['function'],
  })
})

it('keeps Enter as a newline in the mobile first message', async () => {
  const start = vi.fn<NewSessionState['start']>()
  root = createRoot(container)
  act(() => root?.render(<StatefulKeyboardForm isMobile start={start} />))
  const textarea = container.querySelector<HTMLTextAreaElement>(
    '[aria-label="First message"]',
  )
  if (textarea === null) throw new Error('First message field was not rendered')
  await act(async () => {
    textarea.focus()
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    message: textarea.value,
    starts: start.mock.calls,
  })
  expect(snapshot()).toEqual({ message: '\n', starts: [] })
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

it('closes directory suggestions when clicking the open trigger', async () => {
  renderForm(makeState())
  const directoryButton = requireButton('.new-session-directory-button')

  await act(async () => {
    await userEvent.click(directoryButton)
  })
  const opened = {
    expanded: directoryButton.getAttribute('aria-expanded'),
    dialogCount: container.querySelectorAll('[aria-label="Choose a directory"]')
      .length,
  }

  await act(async () => {
    await userEvent.click(directoryButton)
  })
  const closed = {
    expanded: directoryButton.getAttribute('aria-expanded'),
    dialogCount: container.querySelectorAll('[aria-label="Choose a directory"]')
      .length,
  }

  const snapshot = () => ({ opened, closed })
  expect(snapshot()).toEqual({
    opened: { expanded: 'true', dialogCount: 1 },
    closed: { expanded: 'false', dialogCount: 0 },
  })
})

it('closes directory suggestions when clicking the header outside its control', () => {
  renderForm(makeState())
  const directoryButton = requireButton('.new-session-directory-button')
  const heading = container.querySelector('h1')
  if (heading === null) throw new Error('Expected new session heading')

  act(() => {
    directoryButton.click()
  })
  const opened = {
    expanded: directoryButton.getAttribute('aria-expanded'),
    dialogCount: container.querySelectorAll('[aria-label="Choose a directory"]')
      .length,
  }

  act(() => {
    heading.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })
  const closed = {
    expanded: directoryButton.getAttribute('aria-expanded'),
    dialogCount: container.querySelectorAll('[aria-label="Choose a directory"]')
      .length,
  }

  const snapshot = () => ({ opened, closed })
  expect(snapshot()).toEqual({
    opened: { expanded: 'true', dialogCount: 1 },
    closed: { expanded: 'false', dialogCount: 0 },
  })
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
  const opened = {
    optionCount: container.querySelectorAll('[role="option"]').length,
    expanded: container
      .querySelector('[aria-label="First message"]')
      ?.getAttribute('aria-expanded'),
  }

  await act(async () => {
    await userEvent.click(heading)
  })

  const closed = () => ({
    optionCount: container.querySelectorAll('[role="option"]').length,
    expanded: container
      .querySelector('[aria-label="First message"]')
      ?.getAttribute('aria-expanded'),
  })
  const snapshot = () => ({ opened, closed: closed() })
  expect(snapshot()).toEqual({
    opened: { optionCount: 2, expanded: 'true' },
    closed: { optionCount: 0, expanded: 'false' },
  })
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

it('passes the launch request to the launch handler', async () => {
  const onStart = vi.fn()
  const request: LaunchSessionRequest = {
    directory: '/Users/example/work/service',
    engine: 'claude',
    worktree: false,
    initialMessage: '',
  }
  const start = vi.fn((callback: (request: LaunchSessionRequest) => void) => {
    callback(request)
  })
  renderForm(
    makeState({
      selectedDirectory: exampleSessionDirectories[0] ?? null,
      start,
    }),
    onStart,
  )
  await act(async () => {
    await userEvent.click(requireButton('.new-session-start'))
  })

  const snapshot = () => ({
    startCalls: start.mock.calls.length,
    launched: onStart.mock.calls,
  })
  expect(snapshot()).toEqual({
    startCalls: 1,
    launched: [[request]],
  })
})
