import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { App } from '#App'
import type { Session, SessionsResponse } from '#types'

const capturedFeed = vi.hoisted(() => ({
  onSessions: null as ((response: unknown) => void) | null,
}))

vi.mock('#session-feed', () => ({
  startSessionFeed: (onSessions: (response: unknown) => void) => {
    capturedFeed.onSessions = onSessions
    return () => {
      capturedFeed.onSessions = null
    }
  },
}))

let container: HTMLDivElement
let root: Root | null = null

function session(
  id: string,
  title: string,
  category: Session['category'] = 'needs_action',
  elapsedSeconds = 8,
  project = 'service refresh',
): Session {
  return {
    id,
    category,
    kind: category === 'needs_action' ? 'Question' : null,
    title,
    preview: null,
    elapsedSeconds,
    unread: false,
    task: null,
    repo: 'example/service',
    project,
    parent: null,
    pullRequest: null,
  }
}

it('keeps the Board updated from the session feed while it is active', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session', 'needs_action', 8, 'alpha'),
      session('session-quiet', 'Quiet session', 'done', 30, 'quiet project'),
    ],
  })

  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
  })
  const projectsBeforeUpdate = Array.from(
    container.querySelectorAll<HTMLElement>('.board-project'),
    (element) => element.getAttribute('aria-label'),
  )

  publishSessions({
    revision: 2,
    sessions: [
      session('session-a', 'Updated session', 'in_progress', 10, 'alpha'),
      session('session-b', 'New session', 'needs_action', 20, 'beta'),
    ],
  })

  const snapshot = () => ({
    projectsBeforeUpdate,
    projectsAfterUpdate: Array.from(
      container.querySelectorAll<HTMLElement>('.board-project'),
      (element) => element.getAttribute('aria-label'),
    ),
    updatedCardTitles: Array.from(
      container.querySelectorAll<HTMLElement>('.board-card-title'),
      (element) => element.textContent,
    ),
    activeView: readActiveView(),
  })

  expect(snapshot()).toEqual({
    projectsBeforeUpdate: ['alpha'],
    projectsAfterUpdate: ['beta', 'alpha'],
    updatedCardTitles: ['New session', 'Updated session'],
    activeView: 'Board',
  })
})

it('hides quiet projects by default and lets the filter reveal them', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-active', 'Waiting session', 'needs_action', 8, 'active'),
      session('session-quiet', 'Finished session', 'done', 30, 'quiet'),
    ],
  })
  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
  })

  const visibleBeforeToggle = Array.from(
    container.querySelectorAll<HTMLElement>('.board-project'),
    (element) => element.getAttribute('aria-label'),
  )
  const quietToggle = container.querySelector<HTMLInputElement>(
    '.board-quiet-toggle input',
  )
  act(() => {
    quietToggle?.click()
  })

  const snapshot = () => ({
    visibleBeforeToggle,
    visibleAfterToggle: Array.from(
      container.querySelectorAll<HTMLElement>('.board-project'),
      (element) => element.getAttribute('aria-label'),
    ),
    toggleChecked: quietToggle?.checked,
    label: container.querySelector('.board-quiet-toggle')?.textContent,
  })

  expect(snapshot()).toEqual({
    visibleBeforeToggle: ['active'],
    visibleAfterToggle: ['active', 'quiet'],
    toggleChecked: false,
    label: 'Hide quiet projects (1)',
  })
})

function readSelectedTitle() {
  return (
    container.querySelector<HTMLElement>(
      '[data-session-row][aria-current="true"] .session-title',
    )?.textContent ?? null
  )
}

function readSessionTitles() {
  return Array.from(
    container.querySelectorAll<HTMLElement>(
      '[data-session-row] .session-title',
    ),
    (element) => element.textContent,
  )
}

function readActiveView() {
  const active = container.querySelector<HTMLButtonElement>(
    '.view-switch-button[aria-pressed="true"]',
  )
  if (active === null) {
    return ''
  }

  const label = active.textContent.trim()
  return label.startsWith('Inbox') ? 'Inbox' : label
}

function renderApp() {
  const appRoot = createRoot(container)
  root = appRoot
  act(() => {
    appRoot.render(<App />)
  })
}

function publishSessions(response: SessionsResponse) {
  const onSessions = capturedFeed.onSessions
  if (onSessions === null) {
    throw new Error('The session feed has not started')
  }

  act(() => {
    onSessions(response)
  })
}

function dispatchKey(
  target: EventTarget,
  key: string,
  options: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
    ...options,
  })
  target.dispatchEvent(event)
  return event
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  capturedFeed.onSessions = null
})

afterEach(() => {
  const currentRoot = root
  if (currentRoot !== null) {
    act(() => {
      currentRoot.unmount()
    })
  }
  root = null
  container.remove()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('moves the selected App row with j and k and scrolls it into view', () => {
  const scrollIntoView = vi.spyOn(HTMLElement.prototype, 'scrollIntoView')
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  scrollIntoView.mockClear()

  let jEvent: KeyboardEvent | null = null
  act(() => {
    jEvent = dispatchKey(window, 'j')
  })
  const afterJ = readSelectedTitle()

  let kEvent: KeyboardEvent | null = null
  act(() => {
    kEvent = dispatchKey(window, 'k')
  })

  const snapshot = () => ({
    afterJ,
    afterK: readSelectedTitle(),
    defaultPrevented: [jEvent?.defaultPrevented, kEvent?.defaultPrevented],
    scrollOptions: scrollIntoView.mock.calls.map(([options]) => options),
  })

  expect(snapshot()).toEqual({
    afterJ: 'Second session',
    afterK: 'First session',
    defaultPrevented: [true, true],
    scrollOptions: [{ block: 'nearest' }, { block: 'nearest' }],
  })
})

it('ignores shortcuts from input fields and events with modifiers', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  const input = document.createElement('input')
  const textarea = document.createElement('textarea')
  const select = document.createElement('select')
  const editable = document.createElement('div')
  editable.contentEditable = 'true'
  document.body.append(input, textarea, select, editable)

  const events: KeyboardEvent[] = []
  act(() => {
    events.push(dispatchKey(input, 'j'))
    events.push(dispatchKey(textarea, 'j'))
    events.push(dispatchKey(select, 'j'))
    events.push(dispatchKey(editable, 'j'))
    events.push(dispatchKey(window, 'j', { altKey: true }))
    events.push(dispatchKey(window, 'j', { ctrlKey: true }))
    events.push(dispatchKey(window, 'j', { metaKey: true }))
    events.push(dispatchKey(window, 'j', { shiftKey: true }))
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    defaultPrevented: events.map((event) => event.defaultPrevented),
  })

  expect(snapshot()).toEqual({
    selectedTitle: 'First session',
    defaultPrevented: [false, false, false, false, false, false, false, false],
  })
  input.remove()
  textarea.remove()
  select.remove()
  editable.remove()
})

it('does not move the hidden Inbox selection while Board is active', () => {
  const scrollIntoView = vi.spyOn(HTMLElement.prototype, 'scrollIntoView')
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  scrollIntoView.mockClear()

  const boardButton = container.querySelector<HTMLButtonElement>(
    '.view-switch-button[aria-pressed="false"]',
  )
  act(() => {
    boardButton?.click()
  })
  const viewWhileBoardIsActive = readActiveView()

  let event: KeyboardEvent | null = null
  act(() => {
    event = dispatchKey(window, 'j')
  })

  const inboxButton = container.querySelector<HTMLButtonElement>(
    '.view-switch-button[aria-pressed="false"]',
  )
  act(() => {
    inboxButton?.click()
  })

  const snapshot = () => ({
    viewWhileBoardIsActive,
    defaultPrevented: event?.defaultPrevented,
    viewAfterReturn: readActiveView(),
    selectedTitle: readSelectedTitle(),
    scrollOptions: scrollIntoView.mock.calls.map(([options]) => options),
  })

  expect(snapshot()).toEqual({
    viewWhileBoardIsActive: 'Board',
    defaultPrevented: false,
    viewAfterReturn: 'Inbox',
    selectedTitle: 'First session',
    scrollOptions: [{ block: 'nearest' }],
  })
})

it('shows additional in-progress rows when Show more is clicked', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First working session', 'in_progress'),
      session('session-b', 'Second working session', 'in_progress'),
      session('session-c', 'Third working session', 'in_progress'),
    ],
  })

  const showMoreButton =
    container.querySelector<HTMLButtonElement>('.show-more-button')
  const expandedBeforeClick = showMoreButton?.getAttribute('aria-expanded')
  act(() => {
    showMoreButton?.click()
  })

  const snapshot = () => ({
    expandedBeforeClick,
    titlesAfterClick: readSessionTitles(),
    showMoreAfterClick: container.querySelector('.show-more-button') !== null,
  })

  expect(snapshot()).toEqual({
    expandedBeforeClick: 'false',
    titlesAfterClick: [
      'First working session',
      'Second working session',
      'Third working session',
    ],
    showMoreAfterClick: false,
  })
})

it('selects the first remaining row when an update removes the selected one', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Selected session'),
      session('session-later', 'Future category', 'later'),
    ],
  })

  act(() => {
    dispatchKey(window, 'j')
  })
  const selectedBeforeUpdate = readSelectedTitle()

  publishSessions({
    revision: 2,
    sessions: [
      session('session-a', 'First session'),
      session('session-c', 'Replacement session'),
      session('session-later-updated', 'Future category update', 'later'),
    ],
  })

  const snapshot = () => ({
    selectedBeforeUpdate,
    titlesAfterUpdate: readSessionTitles(),
    selectedAfterUpdate: readSelectedTitle(),
  })

  expect(snapshot()).toEqual({
    selectedBeforeUpdate: 'Selected session',
    titlesAfterUpdate: ['First session', 'Replacement session'],
    selectedAfterUpdate: 'First session',
  })
})

it('does not scroll when an update keeps the selected row', () => {
  const scrollIntoView = vi.spyOn(HTMLElement.prototype, 'scrollIntoView')
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  scrollIntoView.mockClear()

  publishSessions({
    revision: 2,
    sessions: [
      session('session-a', 'Updated first session'),
      session('session-b', 'Updated second session'),
    ],
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    scrollOptions: scrollIntoView.mock.calls.map(([options]) => options),
  })

  expect(snapshot()).toEqual({
    selectedTitle: 'Updated first session',
    scrollOptions: [],
  })
})

it('advances elapsed time for all rows with one shared timer', async () => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-11-10T00:00:00.000Z'))
  const intervalSpy = vi.spyOn(window, 'setInterval')

  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session', 'needs_action', 8),
      session('session-b', 'Working session', 'in_progress', 30),
    ],
  })
  const before = Array.from(
    container.querySelectorAll<HTMLElement>('.session-age'),
    (element) => element.textContent,
  )

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000)
  })

  const afterFirstTick = Array.from(
    container.querySelectorAll<HTMLElement>('.session-age'),
    (element) => element.textContent,
  )

  publishSessions({
    revision: 2,
    sessions: [
      session('session-a', 'First session', 'needs_action', 9),
      session('session-b', 'Working session', 'in_progress', 31),
    ],
  })
  const afterUpdate = Array.from(
    container.querySelectorAll<HTMLElement>('.session-age'),
    (element) => element.textContent,
  )

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000)
  })

  const snapshot = () => ({
    before,
    afterFirstTick,
    afterUpdate,
    afterSecondTick: Array.from(
      container.querySelectorAll<HTMLElement>('.session-age'),
      (element) => element.textContent,
    ),
    intervalCount: intervalSpy.mock.calls.length,
  })

  expect(snapshot()).toEqual({
    before: ['8s', 'Working · 30s'],
    afterFirstTick: ['9s', 'Working · 31s'],
    afterUpdate: ['9s', 'Working · 31s'],
    afterSecondTick: ['10s', 'Working · 32s'],
    intervalCount: 1,
  })
})
