import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

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
  parent: Session['parent'] = null,
  ancestorSessionIds = parent === null ? [] : [parent.id],
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
    parent,
    ancestorSessionIds,
    pullRequest: null,
  }
}

function projectSessions(): Session[] {
  return [
    session('session-pm', 'Project manager', 'in_progress', 30),
    session(
      'session-worker',
      'Handle an alert',
      'needs_action',
      8,
      'service refresh',
      {
        id: 'session-owner',
        title: 'Project owner',
      },
      ['session-pm', 'session-owner'],
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
    target: 'project PM row',
    selector: '[aria-label="Open service refresh PM session in Inbox"]',
    expectedTitle: 'Project manager',
    activation: 'click',
  },
  {
    target: 'project PM row',
    selector: '[aria-label="Open service refresh PM session in Inbox"]',
    expectedTitle: 'Project manager',
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
    publishSessions({ revision: 1, sessions: projectSessions() })
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

it('disables a PM row when its project has no PM session', () => {
  renderApp()
  publishSessions({
    revision: 1,
    sessions: [session('session-alone', 'Unmanaged session')],
  })
  act(() => {
    dispatchKey(window, 'B', { shiftKey: true })
  })

  const pmButton = container.querySelector<HTMLButtonElement>('.pm-project')
  const disabled = pmButton?.disabled
  act(() => {
    pmButton?.click()
  })

  const snapshot = () => ({ disabled, activeView: readActiveView() })

  expect(snapshot()).toEqual({
    disabled: true,
    activeView: 'Board',
  })
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

function mockSessionActionFetch(input: RequestInfo | URL) {
  const response =
    requestUrl(input) === '/api/sessions/later-options'
      ? new Response(JSON.stringify(laterPresetTimes), { status: 200 })
      : new Response(null, { status: 204 })
  return Promise.resolve(response)
}

function flushAsyncState() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 0))
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
  vi.stubGlobal('fetch', () => new Promise(() => undefined))
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

  const snapshot = () => ({ menuItems, requests: fetchMock.mock.calls })

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

  expect(fetchMock.mock.calls).toEqual([
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
  const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(response)
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
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockReturnValueOnce(firstResponse)
    .mockReturnValueOnce(secondResponse)
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
    .mockResolvedValue(new Response(null, { status: 204 }))
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

  const snapshot = () => fetchMock.mock.calls
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
    requests: fetchMock.mock.calls,
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
