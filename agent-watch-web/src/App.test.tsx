import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { App } from '#App'
import { readSavedInboxViews } from '#saved-inbox-views'
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
let mobileViewportMatches = false
const nativeMatchMedia = window.matchMedia.bind(window)
const mediaQueries = new Set<MediaQueryList>()

function matchMediaMock(media: string): MediaQueryList {
  const query = nativeMatchMedia(media)
  Object.defineProperty(query, 'matches', {
    configurable: true,
    get: () => mobileViewportMatches,
  })
  mediaQueries.add(query)
  return query
}

function setMobileViewport(matches: boolean) {
  mobileViewportMatches = matches
  for (const query of mediaQueries) {
    query.dispatchEvent(
      new MediaQueryListEvent('change', {
        matches,
        media: query.media,
      }),
    )
  }
}

function session(
  id: string,
  title: string,
  category: Session['category'] = 'needs_action',
  elapsedSeconds = 8,
  project = 'sample project',
  parent: Session['parent'] = null,
  ancestorSessionIds = parent === null ? [] : [parent.id],
  workType: string | null = null,
): Session {
  return {
    id,
    workType,
    category,
    isPaused: false,
    kind: category === 'needs_action' ? 'Question' : null,
    title,
    preview: null,
    elapsedSeconds,
    unread: false,
    task: null,
    repo: 'example/service',
    project,
    parent,
    ancestorSessionIds,
    pullRequest: null,
  }
}

function projectSessions(): Session[] {
  return [
    session(
      'session-guide-old',
      'Review the release notes',
      'in_progress',
      30,
      'sample project',
      null,
      [],
      'release-guide',
    ),
    session(
      'session-worker',
      'Handle an alert',
      'needs_action',
      8,
      'sample project',
      {
        id: 'session-parent',
        title: 'Session parent',
      },
      ['session-guide-old', 'session-parent'],
    ),
    session(
      'session-guide-new',
      'Prepare a launch checklist',
      'needs_action',
      8,
      'sample project',
      null,
      [],
      'release-guide',
    ),
  ]
}

const openSessionCases = [
  {
    target: 'Board card',
    selector: '[aria-label="Open in Inbox: Handle an alert"]',
    expectedTitle: 'Handle an alert',
    activation: 'click',
  },
  {
    target: 'Board card',
    selector: '[aria-label="Open in Inbox: Handle an alert"]',
    expectedTitle: 'Handle an alert',
    activation: 'keyboard',
  },
  {
    target: 'configured work type row',
    selector: '[aria-label="Open sample project Guides session in Inbox"]',
    expectedTitle: 'Prepare a launch checklist',
    activation: 'click',
  },
  {
    target: 'configured work type row',
    selector: '[aria-label="Open sample project Guides session in Inbox"]',
    expectedTitle: 'Prepare a launch checklist',
    activation: 'keyboard',
  },
] as const

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

it('toggles between the Board and Inbox with shifted B', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'Sample session')],
  })

  const views: string[] = []
  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
  })
  views.push(readActiveView())
  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
  })
  views.push(readActiveView())

  expect(views).toEqual(['Board', 'Inbox'])
})

it.each(openSessionCases)(
  'opens $target in Inbox with $activation activation',
  async ({ selector, expectedTitle, activation }) => {
    renderApp()
    publishSessions({
      revision: 1,
      sessions: projectSessions(),
      workTypeNavLabels: { 'release-guide': 'Guides' },
    })
    act(() => {
      dispatchKey(window, 'B', { shiftKey: true })
    })

    const target = container.querySelector<HTMLButtonElement>(selector)
    if (activation === 'click') {
      act(() => {
        target?.click()
      })
    } else {
      target?.focus()
      await act(async () => {
        await userEvent.keyboard('{Enter}')
      })
    }

    const snapshot = () => ({
      activeView: readActiveView(),
      selectedTitle: readSelectedTitle(),
    })

    expect(snapshot()).toEqual({
      activeView: 'Inbox',
      selectedTitle: expectedTitle,
    })
  },
)

it('omits the work type list when no display labels are configured', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-guide',
        'Prepare a launch checklist',
        'needs_action',
        8,
        'sample project',
        null,
        [],
        'release-guide',
      ),
    ],
    workTypeNavLabels: {},
  })

  expect(container.querySelectorAll('.work-type-nav')).toHaveLength(0)
})

it('opens a configured Later session from Nav while Later is collapsed', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-later-guide',
        'Defer a report',
        'later',
        8,
        'sample project',
        null,
        [],
        'release-guide',
      ),
    ],
    workTypeNavLabels: { 'release-guide': 'Guides' },
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Open sample project Guides session in Inbox"]',
      )
      ?.click()
  })

  expect(readSelectedTitle()).toBe('Defer a report')
})

const laterPresetTimes = {
  one_hour: { when: '16:00', deadline: '2030-03-04T16:00:00Z' },
  this_evening: { when: '19:00', deadline: '2030-03-04T19:00:00Z' },
  tomorrow: { when: 'Tue 9:00', deadline: '2030-03-05T09:00:00Z' },
  next_week: { when: 'Mon 3/11', deadline: '2030-03-11T09:00:00Z' },
  until_it_changes: { when: 'next activity', deadline: null },
}

function requestUrl(input: RequestInfo | URL) {
  if (typeof input === 'string') {
    return input
  }
  return input instanceof URL ? input.href : input.url
}

function isConversationPaneRequest(input: RequestInfo | URL) {
  const url = requestUrl(input)
  return url.endsWith('/messages') || url.endsWith('/actions')
}

function mockPendingResponse() {
  return new Promise<Response>(() => undefined)
}

function mockSessionActionFetch(input: RequestInfo | URL) {
  const response =
    requestUrl(input) === '/api/sessions/later-options'
      ? new Response(JSON.stringify(laterPresetTimes), { status: 200 })
      : new Response(null, { status: 204 })
  return isConversationPaneRequest(input)
    ? mockPendingResponse()
    : Promise.resolve(response)
}

function flushAsyncState() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 0))
}

function changeHistory(direction: 'back' | 'forward') {
  const popState = waitForPopState()
  window.history[direction]()
  return popState
}

function waitForPopState() {
  return new Promise<void>((resolve) => {
    window.addEventListener(
      'popstate',
      () => {
        resolve()
      },
      { once: true },
    )
  })
}
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

it('prevents file drags anywhere in the app from navigating away', () => {
  renderApp()
  const dispatchFileEvent = (type: 'dragover' | 'drop') => {
    const event = new Event(type, { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'dataTransfer', {
      value: { types: ['Files'], files: [] },
    })
    act(() => {
      document.body.dispatchEvent(event)
    })
    return event
  }

  const snapshot = () => ({
    dragOverPrevented: dispatchFileEvent('dragover').defaultPrevented,
    dropPrevented: dispatchFileEvent('drop').defaultPrevented,
  })
  expect(snapshot()).toEqual({ dragOverPrevented: true, dropPrevented: true })
})

it('allows non-file drags to keep their default behavior', () => {
  renderApp()
  const dispatchTextEvent = (type: 'dragover' | 'drop') => {
    const event = new Event(type, { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'dataTransfer', {
      value: { types: ['text/plain'], files: [] },
    })
    act(() => {
      document.body.dispatchEvent(event)
    })
    return event
  }

  const snapshot = () => ({
    dragOverPrevented: dispatchTextEvent('dragover').defaultPrevented,
    dropPrevented: dispatchTextEvent('drop').defaultPrevented,
  })
  expect(snapshot()).toEqual({ dragOverPrevented: false, dropPrevented: false })
})

function publishSessions(response: SessionsResponse) {
  const onSessions = capturedFeed.onSessions
  if (onSessions === null) {
    throw new Error('The session feed has not started')
  }

  act(() => {
    onSessions({ workTypeNavLabels: {}, ...response })
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

function setPlatform(platform: string) {
  vi.spyOn(navigator, 'platform', 'get').mockReturnValue(platform)
}

it('restores the previous session with browser back and forward', async () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  const rows =
    container.querySelectorAll<HTMLButtonElement>('[data-session-row]')
  act(() => {
    rows[0]?.click()
    rows[1]?.click()
  })
  const secondSessionUrl = window.location.search

  await act(async () => {
    await changeHistory('back')
  })
  const previousSession = readSelectedTitle()
  const previousSessionUrl = window.location.search

  await act(async () => {
    await changeHistory('forward')
  })

  const snapshot = () => ({
    previousSession,
    previousSessionUrl,
    forwardedSession: readSelectedTitle(),
    forwardedUrl: window.location.search,
    secondSessionUrl,
  })

  expect(snapshot()).toEqual({
    previousSession: 'First session',
    previousSessionUrl: '?session=session-a&screen=conversation',
    forwardedSession: 'Second session',
    forwardedUrl: secondSessionUrl,
    secondSessionUrl: '?session=session-b&screen=conversation',
  })
})

it('replaces the session URL while moving with j and k', async () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  act(() => {
    dispatchKey(window, 'j')
  })
  const movedForwardUrl = window.location.search
  act(() => {
    dispatchKey(window, 'k')
    dispatchKey(window, 'j')
  })
  const movedSessionUrl = window.location.search

  await act(async () => {
    await changeHistory('back')
  })

  const snapshot = () => ({
    movedForwardUrl,
    movedSessionUrl,
    selectedTitle: readSelectedTitle(),
    returnedUrl: window.location.search,
  })

  expect(snapshot()).toEqual({
    movedForwardUrl: '?session=session-b&screen=conversation',
    movedSessionUrl: '?session=session-b&screen=conversation',
    selectedTitle: 'First session',
    returnedUrl: '?session=session-a',
  })
})

it('restores the Board from its URL and browser history', async () => {
  window.history.replaceState(null, '', '/?view=board')
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })
  const initialView = readActiveView()

  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
  })
  const inboxUrl = window.location.search

  await act(async () => {
    await changeHistory('back')
  })

  const snapshot = () => ({
    initialView,
    inboxUrl,
    restoredView: readActiveView(),
    restoredUrl: window.location.search,
  })
  expect(snapshot()).toEqual({
    initialView: 'Board',
    inboxUrl: '?session=session-a',
    restoredView: 'Board',
    restoredUrl: '?session=session-a&view=board',
  })
})

it('opens the session and screen encoded in a mobile URL', () => {
  window.history.replaceState(
    null,
    '',
    '/?session=session-b&screen=conversation',
  )
  mobileViewportMatches = true
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    screen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
    conversationTitle: container.querySelector('.conversation-pane h1')
      ?.textContent,
    url: window.location.search,
  })

  expect(snapshot()).toEqual({
    selectedTitle: 'Second session',
    screen: 'conversation',
    conversationTitle: 'Second session',
    url: '?session=session-b&screen=conversation',
  })
})

it('uses browser back when Inbox is the previous entry for the same session', async () => {
  window.history.replaceState(null, '', '/?session=session-b')
  window.history.pushState(null, '', '/?session=session-a')
  mobileViewportMatches = true
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })

  const returnedFromConversation = waitForPopState()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Back to Inbox"]')
      ?.click()
  })
  await act(async () => {
    await returnedFromConversation
  })
  const inboxUrl = window.location.search

  await act(async () => {
    await changeHistory('back')
  })

  const snapshot = () => ({
    inboxUrl,
    previousUrl: window.location.search,
    previousSelection: readSelectedTitle(),
    previousScreen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
  })
  expect(snapshot()).toEqual({
    inboxUrl: '?session=session-a',
    previousUrl: '?session=session-b',
    previousSelection: 'Second session',
    previousScreen: 'inbox',
  })
})

it('replaces the conversation URL when it has no matching Inbox history', async () => {
  window.history.replaceState(null, '', '/?session=session-b')
  window.history.pushState(null, '', '/?session=session-a&screen=conversation')
  mobileViewportMatches = true
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Back to Inbox"]')
      ?.click()
  })
  const inboxUrl = window.location.search

  await act(async () => {
    await changeHistory('back')
  })

  const snapshot = () => ({
    inboxUrl,
    previousUrl: window.location.search,
    previousSelection: readSelectedTitle(),
    previousScreen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
  })
  expect(snapshot()).toEqual({
    inboxUrl: '?session=session-a',
    previousUrl: '?session=session-b',
    previousSelection: 'Second session',
    previousScreen: 'inbox',
  })
})

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  vi.stubGlobal('fetch', () => new Promise(() => undefined))
  vi.stubGlobal('matchMedia', matchMediaMock)
  setPlatform('Linux x86_64')
  mobileViewportMatches = false
  window.history.replaceState(null, '', '/')
  window.localStorage.removeItem('agent-watch-web:saved-inbox-views')
  mediaQueries.clear()
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
  window.localStorage.removeItem('agent-watch:navigation-width')
  container.remove()
  window.localStorage.removeItem('agent-watch-web:saved-inbox-views')
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('saves a filtered view, restores it, and opens it from navigation', async () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-sample',
        'Sample project session',
        'needs_action',
        8,
        'sample project',
      ),
      session(
        'session-worker',
        'Worker project session',
        'needs_action',
        8,
        'worker',
      ),
    ],
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')?.click()
  })
  const filterInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (filterInput === null) throw new Error('Expected the filter input')
  await act(async () => {
    await userEvent.fill(filterInput, 'project:worker')
    await userEvent.keyboard('{Enter}')
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-save-view')?.click()
  })
  const nameInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Saved view name"]',
  )
  if (nameInput === null) throw new Error('Expected the saved view name input')
  await act(async () => {
    await userEvent.fill(nameInput, 'Worker reviews')
    await userEvent.keyboard('{Enter}')
  })

  const persistedView = readSavedInboxViews().views.map(({ name, query }) => ({
    name,
    query,
  }))

  act(() => {
    root?.unmount()
  })
  root = null
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-sample',
        'Sample project session',
        'needs_action',
        8,
        'sample project',
      ),
      session(
        'session-worker',
        'Worker project session',
        'needs_action',
        8,
        'worker',
      ),
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.saved-view-link')?.click()
  })

  const snapshot = () => ({
    persistedView,
    savedViewNames: Array.from(
      container.querySelectorAll('.saved-view-name'),
      (element) => element.textContent,
    ),
    selectedView: container
      .querySelector<HTMLButtonElement>('.saved-view-link')
      ?.getAttribute('aria-pressed'),
    activeView: readActiveView(),
    visibleTitles: readSessionTitles(),
  })

  expect(snapshot()).toEqual({
    persistedView: [{ name: 'Worker reviews', query: 'project:worker' }],
    savedViewNames: ['Worker reviews'],
    selectedView: 'true',
    activeView: 'Inbox',
    visibleTitles: ['Worker project session'],
  })
})

it('deletes a saved view from navigation and storage', () => {
  window.localStorage.setItem(
    'agent-watch-web:saved-inbox-views',
    JSON.stringify([
      {
        id: 'saved-view',
        name: 'Sample reviews',
        query: 'project:sample-project',
      },
    ]),
  )
  renderApp()
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Delete Sample reviews view"]',
      )
      ?.click()
  })

  const snapshot = () => ({
    savedViewItems: container.querySelectorAll('.saved-view-item').length,
    storedViews: window.localStorage.getItem(
      'agent-watch-web:saved-inbox-views',
    ),
  })

  expect(snapshot()).toEqual({ savedViewItems: 0, storedViews: '[]' })
})

it('clears the saved view highlight when the inbox filter changes', () => {
  window.localStorage.setItem(
    'agent-watch-web:saved-inbox-views',
    JSON.stringify([
      {
        name: 'Sample reviews',
        query: 'project:sample-project',
      },
    ]),
  )
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-sample',
        'Sample project session',
        'needs_action',
        8,
        'sample-project',
      ),
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.saved-view-link')?.click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Remove Project filter"]')
      ?.click()
  })

  const snapshot = () => ({
    selectedView: container
      .querySelector<HTMLButtonElement>('.saved-view-link')
      ?.getAttribute('aria-pressed'),
    activeView: readActiveView(),
    filterChips: container.querySelector('.inbox-filter-chips'),
  })

  expect(snapshot()).toEqual({
    selectedView: 'false',
    activeView: 'Inbox',
    filterChips: null,
  })
})

it('opens saved views from the mobile inbox', () => {
  window.localStorage.setItem(
    'agent-watch-web:saved-inbox-views',
    JSON.stringify([{ name: 'Worker reviews', query: 'project:worker' }]),
  )
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-sample',
        'Sample project session',
        'needs_action',
        8,
        'sample-project',
      ),
      session(
        'session-worker',
        'Worker project session',
        'needs_action',
        8,
        'worker',
      ),
    ],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.mobile-saved-view-link')
      ?.click()
  })

  const snapshot = () => ({
    viewNames: Array.from(
      container.querySelectorAll('.mobile-saved-view-name'),
      (element) => element.textContent,
    ),
    selectedView: container
      .querySelector<HTMLButtonElement>('.mobile-saved-view-link')
      ?.getAttribute('aria-pressed'),
    visibleTitles: readSessionTitles(),
  })

  expect(snapshot()).toEqual({
    viewNames: ['Worker reviews'],
    selectedView: 'true',
    visibleTitles: ['Worker project session'],
  })
})

it('reports a failed save without adding the view', async () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-worker', 'Worker session', 'needs_action', 8, 'worker'),
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')?.click()
  })
  const filterInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (filterInput === null) throw new Error('Expected the filter input')
  await act(async () => {
    await userEvent.fill(filterInput, 'project:worker')
    await userEvent.keyboard('{Enter}')
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-save-view')?.click()
  })
  const nameInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Saved view name"]',
  )
  if (nameInput === null) throw new Error('Expected the saved view name input')
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('Storage is unavailable')
  })
  await act(async () => {
    await userEvent.fill(nameInput, 'Worker reviews')
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    message: container.querySelector('.load-message.is-error')?.textContent,
    savedViews: container.querySelectorAll('.saved-view-item').length,
  })

  expect(snapshot()).toEqual({
    message: 'Could not save the view.',
    savedViews: 0,
  })
})

it('reports a failed delete and keeps the saved view visible', () => {
  window.localStorage.setItem(
    'agent-watch-web:saved-inbox-views',
    JSON.stringify([
      { name: 'Sample reviews', query: 'project:sample-project' },
    ]),
  )
  renderApp()
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('Storage is unavailable')
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Delete Sample reviews view"]',
      )
      ?.click()
  })

  const snapshot = () => ({
    message: container.querySelector('.load-message.is-error')?.textContent,
    savedViewNames: Array.from(
      container.querySelectorAll('.saved-view-name'),
      (element) => element.textContent,
    ),
    stored: readSavedInboxViews().views.map(({ name, query }) => ({
      name,
      query,
    })),
  })

  expect(snapshot()).toEqual({
    message: 'Could not delete the view.',
    savedViewNames: ['Sample reviews'],
    stored: [{ name: 'Sample reviews', query: 'project:sample-project' }],
  })
})

it('preserves views written to storage after this tab was opened', async () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-worker', 'Worker session', 'needs_action', 8, 'worker'),
    ],
  })
  window.localStorage.setItem(
    'agent-watch-web:saved-inbox-views',
    JSON.stringify([{ name: 'Existing', query: 'project:existing' }]),
  )
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')?.click()
  })
  const filterInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (filterInput === null) throw new Error('Expected the filter input')
  await act(async () => {
    await userEvent.fill(filterInput, 'project:worker')
    await userEvent.keyboard('{Enter}')
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-save-view')?.click()
  })
  const nameInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Saved view name"]',
  )
  if (nameInput === null) throw new Error('Expected the saved view name input')
  await act(async () => {
    await userEvent.fill(nameInput, 'Worker reviews')
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () =>
    readSavedInboxViews().views.map(({ name, query }) => ({ name, query }))

  expect(snapshot()).toEqual([
    { name: 'Existing', query: 'project:existing' },
    { name: 'Worker reviews', query: 'project:worker' },
  ])
})

it('switches between the Inbox and conversation at the mobile breakpoint', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'Review the retry policy')],
  })

  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })

  const mobileConversation = {
    layout: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-layout'),
    screen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
    title: container.querySelector('.conversation-pane h1')?.textContent,
    navCount: container.querySelectorAll('.agent-watch-nav').length,
    navSeparatorCount: container.querySelectorAll(
      '[aria-label="Resize navigation and content"]',
    ).length,
  }

  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Back to Inbox"]')
      ?.click()
  })
  act(() => {
    setMobileViewport(false)
  })

  const snapshot = () => ({
    mobileConversation,
    mobileInboxScreen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
    desktopLayout: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-layout'),
    desktopNavCount: container.querySelectorAll('.agent-watch-nav').length,
    desktopNavSeparatorCount: container.querySelectorAll(
      '[aria-label="Resize navigation and content"]',
    ).length,
  })
  expect(snapshot()).toEqual({
    mobileConversation: {
      layout: 'mobile',
      screen: 'conversation',
      title: 'Review the retry policy',
      navCount: 0,
      navSeparatorCount: 0,
    },
    mobileInboxScreen: 'inbox',
    desktopLayout: 'desktop',
    desktopNavCount: 1,
    desktopNavSeparatorCount: 1,
  })
})

it('restores the desktop navigation width after keyboard resizing', () => {
  const storageKey = 'agent-watch:navigation-width'
  window.localStorage.removeItem(storageKey)
  container.style.width = '1200px'
  renderApp()

  const resizeNavigation = () =>
    container.querySelector<HTMLDivElement>(
      '[aria-label="Resize navigation and content"]',
    )
  const separator = resizeNavigation()
  if (separator === null) throw new Error('Expected the navigation separator')

  act(() => {
    separator.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'ArrowRight',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const resizedWidth = separator.getAttribute('aria-valuenow')
  const storedWidth = window.localStorage.getItem(storageKey)
  act(() => {
    root?.unmount()
  })
  root = null
  renderApp()

  const restoredSeparator = resizeNavigation()
  const snapshot = () => ({
    resizedWidth,
    storedWidth,
    restoredWidth: restoredSeparator?.getAttribute('aria-valuenow'),
    navigationPaneWidth: container.querySelector<HTMLDivElement>(
      '.resizable-columns-first-pane',
    )?.style.flexBasis,
  })

  expect(snapshot()).toEqual({
    resizedWidth: '250',
    storedWidth: '250',
    restoredWidth: '250',
    navigationPaneWidth: '250px',
  })
})

it('shows desktop review guidance without mounting crit on mobile', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      {
        ...session('session-review', 'Review the retry policy'),
        kind: 'Review',
        critUrl: 'http://127.0.0.1:5000/review/sample',
      },
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-crit-toggle')
      ?.click()
  })
  const desktopIframeCount = container.querySelectorAll('iframe').length
  const desktopResizers = Array.from(
    container.querySelectorAll<HTMLElement>('[role="separator"]'),
    (separator) => separator.getAttribute('aria-label'),
  )
  act(() => {
    setMobileViewport(true)
  })

  const snapshot = () => ({
    guidance: container.querySelector('.mobile-review-note')?.textContent,
    iframeCount: container.querySelectorAll('iframe').length,
    critButtonCount: container.querySelectorAll('.conversation-crit-toggle')
      .length,
    resizerCount: container.querySelectorAll('[role="separator"]').length,
    desktopIframeCount,
    desktopResizers,
  })
  expect(snapshot()).toEqual({
    guidance: 'Open this session on your Mac to review it in crit.',
    iframeCount: 0,
    critButtonCount: 0,
    resizerCount: 0,
    desktopIframeCount: 1,
    desktopResizers: [
      'Resize navigation and content',
      'Resize session list and conversation',
      'Resize conversation and crit',
    ],
  })
})

it('shows desktop review guidance when a Review session has no crit URL', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      {
        ...session('session-review', 'Review the retry policy'),
        kind: 'Review',
      },
    ],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })

  const snapshot = () => ({
    guidance: container.querySelector('.mobile-review-note')?.textContent,
    iframeCount: container.querySelectorAll('iframe').length,
  })
  expect(snapshot()).toEqual({
    guidance: 'Open this session on your Mac to review it in crit.',
    iframeCount: 0,
  })
})

it('ignores desktop keyboard shortcuts at the mobile breakpoint', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    dispatchKey(window, 'j')
    dispatchKey(window, 'l')
    dispatchKey(window, 'B', { shiftKey: true })
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    laterMenuCount: container.querySelectorAll('.later-menu').length,
    boardViewCount: container.querySelectorAll('.board-view').length,
  })
  expect(snapshot()).toEqual({
    selectedTitle: 'First session',
    laterMenuCount: 0,
    boardViewCount: 0,
  })
})

it('resizes the mobile shell with the visual viewport', () => {
  const visualViewport = window.visualViewport
  if (visualViewport === null) {
    throw new Error('Expected a visual viewport')
  }
  let viewportHeight = 800
  vi.spyOn(visualViewport, 'height', 'get').mockImplementation(
    () => viewportHeight,
  )
  renderApp()
  act(() => {
    setMobileViewport(true)
  })
  const initialHeight = document.documentElement.style.getPropertyValue(
    '--mobile-viewport-height',
  )
  viewportHeight = 420
  act(() => {
    visualViewport.dispatchEvent(new Event('resize'))
  })

  const snapshot = () => ({
    initialHeight,
    resizedHeight: document.documentElement.style.getPropertyValue(
      '--mobile-viewport-height',
    ),
  })
  expect(snapshot()).toEqual({
    initialHeight: '800px',
    resizedHeight: '420px',
  })
})

it('opens a Later session from its mobile TODAY chip', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      {
        ...session('session-today-later', 'Wait for the build', 'later'),
        categoryWhenNotLater: 'needs_action',
        isToday: true,
      },
    ],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.mobile-today-chip')?.click()
  })

  const snapshot = () => ({
    mobileScreen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
    title: container.querySelector('.conversation-pane h1')?.textContent,
    laterExpanded: container
      .querySelector('.later-collapsed-row')
      ?.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    mobileScreen: 'conversation',
    title: 'Wait for the build',
    laterExpanded: 'true',
  })
})

it('opens Later for the session shown in a mobile conversation', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-mobile', 'Reply to the question')],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.mobile-conversation-later')
      ?.click()
  })
  await act(async () => {
    await flushAsyncState()
  })

  const snapshot = () => ({
    menuTitle: container.querySelector('.later-menu-title')?.textContent,
    menuLabel: container
      .querySelector('.later-menu')
      ?.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    menuTitle: 'Later: Reply to the question',
    menuLabel: 'Later: Reply to the question',
  })
})

it('closes the mobile Later menu with Escape', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-mobile', 'Reply to the question')],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.mobile-conversation-later')
      ?.click()
  })
  await act(async () => {
    await flushAsyncState()
  })
  let escapeEvent: KeyboardEvent | undefined
  act(() => {
    escapeEvent = dispatchKey(window, 'Escape')
  })

  const snapshot = () => ({
    menuCount: container.querySelectorAll('.later-menu').length,
    escapePrevented: escapeEvent?.defaultPrevented,
  })
  expect(snapshot()).toEqual({ menuCount: 0, escapePrevented: true })
})

it('returns to the Inbox after submitting Later for a mobile conversation', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'Reply to the question'),
      session('session-b', 'Review another change'),
    ],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.mobile-conversation-later')
      ?.click()
  })
  await act(async () => {
    await flushAsyncState()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '.later-menu-option:not(.later-menu-reset)',
      )
      ?.click()
  })
  await act(async () => {
    await flushAsyncState()
  })
  publishSessions({
    revision: 2,
    sessions: [
      {
        ...session('session-a', 'Reply to the question', 'later'),
        categoryWhenNotLater: 'needs_action',
        laterUntil: '2030-03-04T16:00:00Z',
      },
      session('session-b', 'Review another change'),
    ],
  })

  const snapshot = () => ({
    screen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
    conversationTitle:
      container.querySelector('.conversation-pane h1')?.textContent ?? null,
    inboxSelection: readSelectedTitle(),
    laterRequest: fetchMock.mock.calls
      .filter(
        ([input]) => requestUrl(input) === '/api/sessions/session-a/later',
      )
      .map(([input, init]) => [requestUrl(input), init]),
  })
  expect(snapshot()).toEqual({
    screen: 'inbox',
    conversationTitle: null,
    inboxSelection: 'Review another change',
    laterRequest: [
      [
        '/api/sessions/session-a/later',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{"preset":"one_hour","expectedDeadline":"2030-03-04T16:00:00Z"}',
        },
      ],
    ],
  })
})

it('returns to the Inbox when the mobile conversation disappears from the feed', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'Reply to the question'),
      session('session-b', 'Review another change'),
    ],
  })
  act(() => {
    setMobileViewport(true)
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })

  publishSessions({
    revision: 2,
    sessions: [session('session-b', 'Review another change')],
  })

  const snapshot = () => ({
    screen: container
      .querySelector('.agent-watch-shell')
      ?.getAttribute('data-mobile-screen'),
    conversationTitle:
      container.querySelector('.conversation-pane h1')?.textContent ?? null,
    inboxSelection: readSelectedTitle(),
  })
  expect(snapshot()).toEqual({
    screen: 'inbox',
    conversationTitle: null,
    inboxSelection: 'Review another change',
  })
})

it('keeps the desktop Later summary hidden when empty and restores its markers', () => {
  renderApp()
  publishSessions({ revision: 1, sessions: [] })
  const emptySummary =
    container.querySelector('.later-next-return')?.textContent
  publishSessions({
    revision: 2,
    sessions: [session('session-later', 'Wait for the build', 'later')],
  })
  const collapsedLabel = container
    .querySelector('.later-next-return')
    ?.textContent.trim()
  act(() => {
    container.querySelector<HTMLButtonElement>('.later-collapsed-row')?.click()
  })
  const expandedLabel = container
    .querySelector('.later-next-return')
    ?.textContent.trim()

  const snapshot = () => ({ emptySummary, collapsedLabel, expandedLabel })
  expect(snapshot()).toEqual({
    emptySummary: undefined,
    collapsedLabel: 'Next activity ▸',
    expandedLabel: 'Next activity ▾',
  })
})

it('keeps Later, In progress, and Done collapsed until opened on mobile', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-needs-action', 'Reply to the question'),
      session('session-later', 'Wait for the build', 'later'),
      session('session-progress', 'Update the client', 'in_progress'),
      session('session-done', 'Document the change', 'done'),
    ],
  })
  act(() => {
    setMobileViewport(true)
  })

  const visibleTitles = () =>
    Array.from(
      container.querySelectorAll<HTMLElement>(
        '.inbox-view [data-session-row] .session-title',
      ),
      (element) => element.textContent,
    )
  const initiallyVisible = visibleTitles()
  act(() => {
    container.querySelector<HTMLButtonElement>('.later-collapsed-row')?.click()
  })
  const afterLater = visibleTitles()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.mobile-in-progress-row')
      ?.click()
  })
  const afterInProgress = visibleTitles()
  act(() => {
    container.querySelector<HTMLButtonElement>('.mobile-done-row')?.click()
  })
  const afterDone = visibleTitles()

  const snapshot = () => ({
    initiallyVisible,
    afterLater,
    afterInProgress,
    afterDone,
  })
  expect(snapshot()).toEqual({
    initiallyVisible: ['Reply to the question'],
    afterLater: ['Reply to the question', 'Wait for the build'],
    afterInProgress: [
      'Reply to the question',
      'Wait for the build',
      'Update the client',
    ],
    afterDone: [
      'Reply to the question',
      'Wait for the build',
      'Update the client',
      'Document the change',
    ],
  })
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

it('limits keyboard navigation to sessions matching the Inbox filters', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-sample',
        'Sample project session',
        'needs_action',
        8,
        'sample project',
      ),
      session(
        'session-worker',
        'Worker project session',
        'needs_action',
        8,
        'worker',
      ),
    ],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')?.click()
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    if (input !== null && valueSetter !== undefined) {
      Reflect.apply(valueSetter, input, ['project:worker'])
      input.dispatchEvent(new Event('input', { bubbles: true }))
      container
        .querySelector('form.inbox-filter-search-row')
        ?.dispatchEvent(
          new Event('submit', { bubbles: true, cancelable: true }),
        )
    }
  })

  act(() => {
    dispatchKey(window, 'j')
  })

  expect(readSelectedTitle()).toEqual('Worker project session')
})

it('opens the command palette with Ctrl-K on other platforms', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'Review the retry policy')],
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')?.click()
  })
  const filterInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (filterInput === null)
    throw new Error('Inbox filter input was not rendered')

  act(() => {
    dispatchKey(filterInput, 'k', { ctrlKey: true })
  })
  const opened = container.querySelector('[aria-label="Jump to"]') !== null
  const paletteInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (paletteInput === null) throw new Error('Palette input was not rendered')
  act(() => {
    dispatchKey(paletteInput, 'Escape')
  })

  const snapshot = () => ({
    opened,
    paletteClosed: container.querySelector('[aria-label="Jump to"]') === null,
    filterStillOpen:
      container.querySelector('[aria-label="Filter inbox"]') !== null,
  })
  expect(snapshot()).toEqual({
    opened: true,
    paletteClosed: true,
    filterStillOpen: true,
  })
})

it('leaves Ctrl-K available to a focused field on macOS', () => {
  setPlatform('MacIntel')
  renderApp()
  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')?.click()
  })
  const filterInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (filterInput === null)
    throw new Error('Inbox filter input was not rendered')

  let defaultPrevented = false
  act(() => {
    defaultPrevented = dispatchKey(filterInput, 'k', {
      ctrlKey: true,
    }).defaultPrevented
  })

  const snapshot = () => ({
    paletteOpen: container.querySelector('[aria-label="Jump to"]') !== null,
    defaultPrevented,
    focusedInput: document.activeElement?.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    paletteOpen: false,
    defaultPrevented: false,
    focusedInput: 'Inbox filter query',
  })
})

it('opens the command palette with Cmd-K on macOS', () => {
  setPlatform('MacIntel')
  renderApp()
  act(() => {
    dispatchKey(window, 'k', { metaKey: true })
  })

  expect(container.querySelector('[aria-label="Jump to"]') !== null).toEqual(
    true,
  )
})

it('opens the command palette from the navigation shortcut', async () => {
  setPlatform('MacIntel')
  renderApp()
  const shortcut = container.querySelector<HTMLButtonElement>(
    '[aria-label="Jump to with Command K"]',
  )
  if (shortcut === null) throw new Error('Navigation shortcut was not rendered')

  await act(async () => {
    await userEvent.click(shortcut)
  })

  const snapshot = () => ({
    dialogRole:
      container.querySelector('[aria-label="Jump to"]')?.getAttribute('role') ??
      null,
    focusedInput: document.activeElement?.getAttribute('aria-label') ?? null,
  })
  expect(snapshot()).toEqual({
    dialogRole: 'dialog',
    focusedInput:
      'Jump to a session, tq task or project, or type > for commands',
  })
})

it('shows Ctrl-K in the navigation on other platforms', () => {
  renderApp()
  const shortcut = container.querySelector<HTMLButtonElement>(
    '.agent-watch-command-key',
  )
  const snapshot = () => ({
    label: shortcut?.textContent,
    ariaLabel: shortcut?.getAttribute('aria-label'),
  })

  expect(snapshot()).toEqual({
    label: 'Ctrl K',
    ariaLabel: 'Jump to with Control K',
  })
})

it('filters the Inbox from a task command in the palette', async () => {
  setPlatform('MacIntel')
  const taskSession = session('session-task', 'Review alert settings')
  taskSession.task = {
    id: 'task-alerts',
    number: 42,
    title: 'Improve notifications',
    priority: 1,
  }
  renderApp()
  publishSessions({ revision: 1, sessions: [taskSession] })
  act(() => {
    dispatchKey(window, 'k', { metaKey: true })
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '>Filter inbox by task #42')
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    activeView: readActiveView(),
    activeFilter: container.querySelector('[aria-label="Active inbox filters"]')
      ?.textContent,
    visibleSessions: readSessionTitles(),
  })
  expect(snapshot()).toEqual({
    activeView: 'Inbox',
    activeFilter: 'tq task #42×',
    visibleSessions: ['Review alert settings'],
  })
})

it('opens a session in the Inbox when selected from the palette on Board', async () => {
  setPlatform('MacIntel')
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-palette', 'Distinctive review session')],
  })
  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
    dispatchKey(window, 'k', { metaKey: true })
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, 'Distinctive review session')
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    view: readActiveView(),
    title: readSelectedTitle(),
  })
  expect(snapshot()).toEqual({
    view: 'Inbox',
    title: 'Distinctive review session',
  })
})

it('opens a selected project on Board at the mobile breakpoint', async () => {
  setPlatform('MacIntel')
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session(
        'session-mobile-project',
        'Review mobile project',
        'needs_action',
      ),
    ],
  })
  act(() => {
    setMobileViewport(true)
    dispatchKey(window, 'k', { metaKey: true })
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '@sample project')
    await userEvent.keyboard('{Enter}')
  })

  const snapshot = () => ({
    boardVisible:
      container.querySelector('[aria-labelledby="board-title"]') !== null,
    focusedProjects: Array.from(
      container.querySelectorAll<HTMLElement>('.board-project'),
      (project) => project.getAttribute('aria-label'),
    ),
  })
  expect(snapshot()).toEqual({
    boardVisible: true,
    focusedProjects: ['sample project'],
  })
})

it('archives each done session from the palette command', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-done-a', 'First finished session', 'done'),
      session('session-done-b', 'Second finished session', 'done'),
    ],
  })
  act(() => {
    dispatchKey(window, 'k', { ctrlKey: true })
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '>Archive all done')
    await userEvent.keyboard('{Enter}')
    await flushAsyncState()
  })

  const archivedUrls = fetchMock.mock.calls
    .map(([request]) => requestUrl(request))
    .filter((url) => url.endsWith('/archive'))
  expect(archivedUrls).toEqual([
    '/api/sessions/session-done-a/archive',
    '/api/sessions/session-done-b/archive',
  ])
})

it('stops the archive batch and reports the failed session', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation((input) =>
      requestUrl(input).endsWith('/archive')
        ? Promise.resolve(new Response(null, { status: 500 }))
        : mockSessionActionFetch(input),
    )
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-failed-archive', 'Unarchivable session', 'done'),
      session('session-not-attempted', 'Later done session', 'done'),
    ],
  })
  act(() => {
    dispatchKey(window, 'k', { ctrlKey: true })
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '>Archive all done')
    await userEvent.keyboard('{Enter}')
    await flushAsyncState()
  })

  const snapshot = () => ({
    error: container.querySelector('.load-message.is-error')?.textContent,
    archiveUrls: fetchMock.mock.calls
      .map(([request]) => requestUrl(request))
      .filter((url) => url.endsWith('/archive')),
  })
  expect(snapshot()).toEqual({
    error: 'Could not archive Unarchivable session.',
    archiveUrls: ['/api/sessions/session-failed-archive/archive'],
  })
})

it('does not start a second archive batch while the first is pending', async () => {
  const archiveRequest = {
    resolve: null as ((response: Response) => void) | null,
  }
  const fetchMock = vi.fn<typeof fetch>().mockImplementation((input) =>
    requestUrl(input).endsWith('/archive')
      ? new Promise<Response>((resolve) => {
          archiveRequest.resolve = resolve
        })
      : mockSessionActionFetch(input),
  )
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-pending-archive', 'Pending archive', 'done')],
  })

  async function openAndRunArchiveCommand() {
    act(() => {
      dispatchKey(window, 'k', { ctrlKey: true })
    })
    const input = container.querySelector<HTMLInputElement>(
      '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
    )
    if (input === null) throw new Error('Palette input was not rendered')
    await act(async () => {
      await userEvent.type(input, '>Archive all done')
      await userEvent.keyboard('{Enter}')
    })
  }

  await openAndRunArchiveCommand()
  await openAndRunArchiveCommand()
  const urlsBeforeCompletion = fetchMock.mock.calls
    .map(([request]) => requestUrl(request))
    .filter((url) => url.endsWith('/archive'))
  const resolveArchive = archiveRequest.resolve
  if (resolveArchive === null)
    throw new Error('Archive request was not started')
  await act(async () => {
    resolveArchive(new Response(null, { status: 204 }))
    await flushAsyncState()
  })

  expect(urlsBeforeCompletion).toEqual([
    '/api/sessions/session-pending-archive/archive',
  ])
})

it('opens Later with L and selects a preset with its number key', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  await act(async () => {
    dispatchKey(window, 'l')
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  const menuItems = Array.from(
    container.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    (item) => item.textContent,
  )
  await act(async () => {
    dispatchKey(window, '3')
    await flushAsyncState()
  })

  const snapshot = () => ({
    menuItems,
    requests: fetchMock.mock.calls.filter(([input]) =>
      ['/api/sessions/later-options', '/api/sessions/session-a/later'].includes(
        requestUrl(input),
      ),
    ),
  })

  expect(snapshot()).toEqual({
    menuItems: [
      'In 1 hour16:001',
      'This evening19:002',
      'TomorrowTue 9:003',
      'Next weekMon 3/114',
      'Until it changesnext activity5',
    ],
    requests: [
      ['/api/sessions/later-options'],
      [
        '/api/sessions/session-a/later',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{"preset":"tomorrow","expectedDeadline":"2030-03-05T09:00:00Z"}',
        },
      ],
    ],
  })
})

it('ignores further Later shortcuts while a preset request is pending', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation((input) =>
      requestUrl(input) === '/api/sessions/session-a/later'
        ? mockPendingResponse()
        : mockSessionActionFetch(input),
    )
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  await act(async () => {
    dispatchKey(window, 'l')
    await flushAsyncState()
  })
  act(() => {
    dispatchKey(window, '1')
    dispatchKey(window, '2')
  })

  const snapshot = () => ({
    menuOpen:
      container.querySelector('[aria-label="Later: First session"]') !== null,
    requests: fetchMock.mock.calls.filter(
      ([input]) => requestUrl(input) === '/api/sessions/session-a/later',
    ),
  })

  expect(snapshot()).toEqual({
    menuOpen: true,
    requests: [
      [
        '/api/sessions/session-a/later',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{"preset":"one_hour","expectedDeadline":"2030-03-04T16:00:00Z"}',
        },
      ],
    ],
  })
})

it('disables Back now and preset shortcuts while a Later request is pending', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation((input) =>
      requestUrl(input) === '/api/sessions/session-later/later'
        ? mockPendingResponse()
        : mockSessionActionFetch(input),
    )
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      {
        ...session('session-later', 'Deferred session', 'later'),
        categoryWhenNotLater: 'needs_action',
        laterUntil: '2030-03-11T09:00:00.000Z',
      },
    ],
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.later-collapsed-row')?.click()
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  await act(async () => {
    dispatchKey(window, 'l')
    await flushAsyncState()
  })
  act(() => {
    dispatchKey(window, '1')
  })
  const backNow = container.querySelector<HTMLButtonElement>(
    '[aria-label="Back now: Deferred session"]',
  )
  act(() => {
    backNow?.click()
    dispatchKey(window, '2')
  })

  const snapshot = () => ({
    menuItemsDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
      (button) => button.disabled,
    ),
    requests: fetchMock.mock.calls.filter(
      ([input]) => requestUrl(input) === '/api/sessions/session-later/later',
    ),
  })

  expect(snapshot()).toEqual({
    menuItemsDisabled: [true, true, true, true, true, true],
    requests: [
      [
        '/api/sessions/session-later/later',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{"preset":"one_hour","expectedDeadline":"2030-03-04T16:00:00Z"}',
        },
      ],
    ],
  })
})

it('accepts a new Later preset after the previous request settles', async () => {
  let resolveFirstRequest: ((response: Response) => void) | undefined
  let shouldHoldFirstRequest = true
  const fetchMock = vi.fn<typeof fetch>().mockImplementation((input) => {
    if (
      requestUrl(input) === '/api/sessions/session-a/later' &&
      shouldHoldFirstRequest
    ) {
      shouldHoldFirstRequest = false
      return new Promise<Response>((resolve) => {
        resolveFirstRequest = resolve
      })
    }
    return mockSessionActionFetch(input)
  })
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  await act(async () => {
    dispatchKey(window, 'l')
    await flushAsyncState()
  })
  act(() => {
    dispatchKey(window, '1')
  })
  const resolveSuccess = resolveFirstRequest
  if (resolveSuccess === undefined) {
    throw new Error('The first Later request did not start')
  }
  await act(async () => {
    resolveSuccess(new Response(null, { status: 204 }))
    await flushAsyncState()
  })
  await act(async () => {
    dispatchKey(window, 'l')
    await flushAsyncState()
  })
  await act(async () => {
    dispatchKey(window, '2')
    await flushAsyncState()
  })

  const snapshot = () =>
    fetchMock.mock.calls.filter(
      ([input]) => requestUrl(input) === '/api/sessions/session-a/later',
    )

  expect(snapshot()).toEqual([
    [
      '/api/sessions/session-a/later',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{"preset":"one_hour","expectedDeadline":"2030-03-04T16:00:00Z"}',
      },
    ],
    [
      '/api/sessions/session-a/later',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{"preset":"this_evening","expectedDeadline":"2030-03-04T19:00:00Z"}',
      },
    ],
  ])
})

it('accepts a new Later preset after a failed request reloads its menu', async () => {
  let resolveFirstRequest: ((response: Response) => void) | undefined
  let resolveReloadRequest: ((response: Response) => void) | undefined
  let laterOptionsRequestCount = 0
  let shouldHoldFirstRequest = true
  const fetchMock = vi.fn<typeof fetch>().mockImplementation((input) => {
    if (requestUrl(input) === '/api/sessions/later-options') {
      laterOptionsRequestCount += 1
      if (laterOptionsRequestCount === 2) {
        return new Promise<Response>((resolve) => {
          resolveReloadRequest = resolve
        })
      }
    }
    if (
      requestUrl(input) === '/api/sessions/session-a/later' &&
      shouldHoldFirstRequest
    ) {
      shouldHoldFirstRequest = false
      return new Promise<Response>((resolve) => {
        resolveFirstRequest = resolve
      })
    }
    return mockSessionActionFetch(input)
  })
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  await act(async () => {
    dispatchKey(window, 'l')
    await flushAsyncState()
  })
  act(() => {
    dispatchKey(window, '1')
  })
  const resolveFailure = resolveFirstRequest
  if (resolveFailure === undefined) {
    throw new Error('The first Later request did not start')
  }
  await act(async () => {
    resolveFailure(new Response(null, { status: 500 }))
    await flushAsyncState()
  })
  const resolveReload = resolveReloadRequest
  if (resolveReload === undefined) {
    throw new Error('The Later return times were not reloaded')
  }
  await act(async () => {
    resolveReload(
      new Response(JSON.stringify(laterPresetTimes), { status: 200 }),
    )
    await flushAsyncState()
  })
  const menuSnapshot = () => ({
    isOpen:
      container.querySelector('[aria-label="Later: First session"]') !== null,
    disabledItems: Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
      (item) => item.disabled,
    ),
  })
  expect(menuSnapshot()).toEqual({
    isOpen: true,
    disabledItems: [false, false, false, false, false],
  })
  await act(async () => {
    dispatchKey(window, '2')
    await flushAsyncState()
  })

  const snapshot = () =>
    fetchMock.mock.calls.filter(
      ([input]) => requestUrl(input) === '/api/sessions/session-a/later',
    )

  expect(snapshot()).toEqual([
    [
      '/api/sessions/session-a/later',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{"preset":"one_hour","expectedDeadline":"2030-03-04T16:00:00Z"}',
      },
    ],
    [
      '/api/sessions/session-a/later',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{"preset":"this_evening","expectedDeadline":"2030-03-04T19:00:00Z"}',
      },
    ],
  ])
})

it('returns a Later session immediately from the menu', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      {
        ...session('session-later', 'Deferred session', 'later'),
        categoryWhenNotLater: 'needs_action',
        laterUntil: '2030-03-11T09:00:00.000Z',
      },
    ],
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.later-collapsed-row')?.click()
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('[data-session-row]')?.click()
  })
  await act(async () => {
    dispatchKey(window, 'l')
    await vi.waitFor(() => {
      expect(
        container.querySelector('[aria-label="Back now: Deferred session"]')
          ?.textContent,
      ).toEqual('Back now')
    })
  })
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Back now: Deferred session"]',
      )
      ?.click()
    await flushAsyncState()
  })

  expect(
    fetchMock.mock.calls.filter(([input]) =>
      [
        '/api/sessions/later-options',
        '/api/sessions/session-later/later',
      ].includes(requestUrl(input)),
    ),
  ).toEqual([
    ['/api/sessions/later-options'],
    [
      '/api/sessions/session-later/later',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{"preset":"back_now"}',
      },
    ],
  ])
})

it.each([
  {
    scenario: 'a failed request',
    response: new Response(null, { status: 503 }),
  },
  {
    scenario: 'an invalid response',
    response: new Response('{', { status: 200 }),
  },
])('disables Later presets after $scenario', async ({ response }) => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation((input) =>
      isConversationPaneRequest(input)
        ? mockPendingResponse()
        : Promise.resolve(response),
    )
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  await act(async () => {
    dispatchKey(window, 'l')
    await flushAsyncState()
  })

  const snapshot = () => ({
    message: container.querySelector('.later-menu-error')?.textContent,
    disabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
      (button) => button.disabled,
    ),
  })

  expect(snapshot()).toEqual({
    message: 'Could not load return times.',
    disabled: [true, true, true, true, true],
  })
})

it('ignores return times from an earlier Later menu request', async () => {
  let resolveFirst!: (response: Response) => void
  let resolveSecond!: (response: Response) => void
  const firstResponse = new Promise<Response>((resolve) => {
    resolveFirst = resolve
  })
  const secondResponse = new Promise<Response>((resolve) => {
    resolveSecond = resolve
  })
  const optionResponses = [firstResponse, secondResponse]
  const fetchMock = vi.fn<typeof fetch>().mockImplementation((input) => {
    if (isConversationPaneRequest(input)) {
      return mockPendingResponse()
    }
    if (requestUrl(input) === '/api/sessions/later-options') {
      return (
        optionResponses.shift() ??
        Promise.resolve(new Response(null, { status: 500 }))
      )
    }
    return Promise.resolve(new Response(null, { status: 204 }))
  })
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'First session'),
      session('session-b', 'Second session'),
    ],
  })

  act(() => {
    dispatchKey(window, 'l')
  })
  act(() => {
    dispatchKey(window, 'Escape')
  })
  act(() => {
    dispatchKey(window, 'j')
  })
  act(() => {
    dispatchKey(window, 'l')
  })
  await act(async () => {
    resolveSecond(
      new Response(
        JSON.stringify({
          ...laterPresetTimes,
          one_hour: { when: '22:00', deadline: '2030-03-04T22:00:00Z' },
        }),
        { status: 200 },
      ),
    )
    await flushAsyncState()
  })
  await act(async () => {
    resolveFirst(
      new Response(
        JSON.stringify({
          ...laterPresetTimes,
          one_hour: { when: '14:00', deadline: '2030-03-04T14:00:00Z' },
        }),
        { status: 200 },
      ),
    )
    await flushAsyncState()
  })

  const returnTime = container.querySelector('.later-menu-when')?.textContent
  expect(returnTime).toEqual('22:00')
})

it('saves a TODAY star from the session row', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation((input) =>
      isConversationPaneRequest(input)
        ? mockPendingResponse()
        : Promise.resolve(new Response(null, { status: 204 })),
    )
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })

  const todayButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Add to TODAY"]',
  )
  await act(async () => {
    todayButton?.click()
    await flushAsyncState()
  })

  const snapshot = () =>
    fetchMock.mock.calls.filter(([input]) =>
      requestUrl(input).endsWith('/today'),
    )
  expect(snapshot()).toEqual([
    [
      '/api/sessions/session-a/today',
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{"isToday":true}',
      },
    ],
  ])
})

it('skips collapsed Later sessions during keyboard navigation', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'Needs action session'),
      session('session-later', 'Later session', 'later'),
      session('session-work', 'In-progress session', 'in_progress'),
    ],
  })

  let event: KeyboardEvent | null = null
  act(() => {
    event = dispatchKey(window, 'j')
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    defaultPrevented: event?.defaultPrevented,
    laterRowsVisible: container.querySelector('.later-session-list') !== null,
  })

  expect(snapshot()).toEqual({
    selectedTitle: 'In-progress session',
    defaultPrevented: true,
    laterRowsVisible: false,
  })
})

it('closes Later when its target session disappears', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(mockSessionActionFetch)
  vi.stubGlobal('fetch', fetchMock)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'Later target'),
      session('session-b', 'Remaining session'),
    ],
  })

  await act(async () => {
    dispatchKey(window, 'l')
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  publishSessions({
    revision: 2,
    sessions: [session('session-b', 'Remaining session')],
  })

  let numberKeyEvent: KeyboardEvent | null = null
  act(() => {
    numberKeyEvent = dispatchKey(window, '3')
  })

  const snapshot = () => ({
    menu: container.querySelector('[role="menu"]'),
    selectedTitle: readSelectedTitle(),
    numberKeyPrevented: numberKeyEvent?.defaultPrevented,
    requests: fetchMock.mock.calls.filter(
      ([input]) => requestUrl(input) === '/api/sessions/later-options',
    ),
  })

  expect(snapshot()).toEqual({
    menu: null,
    selectedTitle: 'Remaining session',
    numberKeyPrevented: false,
    requests: [['/api/sessions/later-options']],
  })
})

it('returns a Later session after its deadline without a feed update', async () => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-11-10T00:00:00.000Z'))
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      {
        ...session('session-later', 'Deferred session', 'later'),
        categoryWhenNotLater: 'needs_action',
        laterUntil: new Date(Date.now() + 1_000).toISOString(),
      },
    ],
  })
  const before = {
    needsActionTitles: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.needs-action-section .session-title',
      ),
      (element) => element.textContent,
    ),
    laterCount: container.querySelector('.later-section-count')?.textContent,
  }

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000)
  })

  const after = {
    needsActionTitles: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.needs-action-section .session-title',
      ),
      (element) => element.textContent,
    ),
    laterCount: container.querySelector('.later-section-count')?.textContent,
  }

  const snapshot = () => ({ before, after })

  expect(snapshot()).toEqual({
    before: { needsActionTitles: [], laterCount: '1' },
    after: { needsActionTitles: ['Deferred session'], laterCount: '0' },
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

it('reloads the selected conversation when the session feed updates', async () => {
  let messageReadCount = 0
  const fetcher = vi.fn<typeof fetch>()
  fetcher.mockImplementation((input) => {
    const path =
      typeof input === 'string'
        ? new URL(input, 'http://localhost').pathname
        : input instanceof URL
          ? input.pathname
          : new URL(input.url).pathname
    const body = path.endsWith('/messages')
      ? [
          {
            type: 'assistant',
            text:
              messageReadCount++ === 0
                ? 'Before feed update'
                : 'After feed update',
          },
        ]
      : {
          message: true,
          answerQuestion: false,
          interrupt: true,
          openInTerminal: true,
        }
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        headers: { 'Content-Type': 'application/json' },
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-a', 'First session')],
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  publishSessions({
    revision: 2,
    sessions: [session('session-a', 'Updated session')],
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    transcript: container.querySelector('.chat-assistant-message')?.textContent,
    requestPaths: fetcher.mock.calls.map(([input]) =>
      typeof input === 'string'
        ? new URL(input, 'http://localhost').pathname
        : input instanceof URL
          ? input.pathname
          : new URL(input.url).pathname,
    ),
  })

  expect(snapshot()).toEqual({
    transcript: 'After feed update',
    requestPaths: [
      '/api/sessions/session-a/messages',
      '/api/sessions/session-a/actions',
      '/api/sessions/session-a/messages',
      '/api/sessions/session-a/actions',
    ],
  })
})

it('opens a known session ID from a conversation message', async () => {
  const linkedId = '33333333-3333-4333-8333-333333333333'
  const fetcher = vi.fn<typeof fetch>()
  fetcher.mockImplementation((input) => {
    const path = new URL(requestUrl(input), 'http://localhost').pathname
    const body = path.endsWith('/messages')
      ? [{ type: 'assistant', text: `Open ${linkedId}` }]
      : {
          message: true,
          answerQuestion: false,
          interrupt: true,
          openInTerminal: true,
        }
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        headers: { 'Content-Type': 'application/json' },
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [
      session('session-a', 'Current session'),
      session(linkedId, 'Linked session', 'done', 12, 'example toolkit'),
    ],
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const link = container.querySelector<HTMLAnchorElement>(
    '.chat-assistant-message a',
  )
  if (link === null) throw new Error('Expected a link to the known session')
  const linkTitle = link.title
  const linkClass = link.className
  const underlineStyle = getComputedStyle(link).textDecorationStyle
  act(() => {
    link.click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    url: window.location.search,
    linkTitle,
    linkClass,
    underlineStyle,
  })
  expect(snapshot()).toEqual({
    selectedTitle: 'Linked session',
    url: `?session=${linkedId}`,
    linkTitle: '',
    linkClass: 'session-link-trigger',
    underlineStyle: 'dotted',
  })
})

it('answers an Inbox question with a number key when the pane is not focused', async () => {
  const requestPath = (input: Parameters<typeof fetch>[0]) =>
    typeof input === 'string'
      ? new URL(input, 'http://localhost').pathname
      : input instanceof URL
        ? input.pathname
        : new URL(input.url).pathname
  const fetcher = vi.fn<typeof fetch>()
  fetcher.mockImplementation((input) => {
    const path = requestPath(input)
    const body = path.endsWith('/messages')
      ? [
          {
            type: 'question',
            question: 'Which retry policy should be used?',
            choices: ['Retry three times', 'Retry once'],
            answer: null,
          },
        ]
      : path.endsWith('/actions')
        ? {
            message: true,
            answerQuestion: true,
            interrupt: true,
            openInTerminal: true,
          }
        : {}
    return Promise.resolve(
      new Response(JSON.stringify(body), {
        headers: { 'Content-Type': 'application/json' },
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-question', 'Choose a retry policy')],
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  let event: KeyboardEvent | null = null
  act(() => {
    event = dispatchKey(window, '1')
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    selectedTitle: readSelectedTitle(),
    defaultPrevented: event?.defaultPrevented,
    requests: fetcher.mock.calls.map(([input, init]) => ({
      path: requestPath(input),
      method: init?.method ?? 'GET',
      body: init?.body ?? null,
    })),
  })

  expect(snapshot()).toEqual({
    selectedTitle: 'Choose a retry policy',
    defaultPrevented: true,
    requests: [
      {
        path: '/api/sessions/session-question/messages',
        method: 'GET',
        body: null,
      },
      {
        path: '/api/sessions/session-question/actions',
        method: 'GET',
        body: null,
      },
      {
        path: '/api/sessions/session-question/answer',
        method: 'POST',
        body: '{"choice":1}',
      },
      {
        path: '/api/sessions/session-question/messages',
        method: 'GET',
        body: null,
      },
      {
        path: '/api/sessions/session-question/actions',
        method: 'GET',
        body: null,
      },
    ],
  })
})
