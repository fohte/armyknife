import '#components/mobile-layout.css'

import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { InboxView } from '#components/inbox-view'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const sessions: Session[] = [
  {
    id: 'sample-needs-action',
    category: 'needs_action',
    isPaused: false,
    kind: 'Question',
    title: 'Check the retry policy',
    preview: null,
    elapsedSeconds: 10,
    unread: true,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-later',
    category: 'later',
    isPaused: false,
    kind: 'Review',
    title: 'Review the changes',
    preview: null,
    elapsedSeconds: 20,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-progress',
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title: 'Add request metrics',
    preview: null,
    elapsedSeconds: 30,
    unread: false,
    task: null,
    repo: 'sample-org/metrics',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-done',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Document request limits',
    preview: null,
    elapsedSeconds: 40,
    unread: false,
    task: null,
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
  {
    id: 'sample-other-project',
    category: 'needs_action',
    isPaused: false,
    kind: 'Error',
    title: 'Inspect worker output',
    preview: null,
    elapsedSeconds: 50,
    unread: true,
    task: null,
    repo: 'sample-org/worker',
    project: 'worker',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  },
]

function renderInbox(props: Partial<ComponentProps<typeof InboxView>> = {}) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <InboxView
        sessions={sessions}
        selectedId={null}
        loaded
        error={null}
        connectionError={null}
        lastUpdatedAt={new Date(2026, 0, 1, 22, 0).getTime()}
        showAllInProgress
        onShowMore={() => undefined}
        onSelect={() => undefined}
        laterExpanded
        onToggleLater={() => undefined}
        onToggleToday={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        isMobile={false}
        inProgressExpanded
        onToggleInProgress={() => undefined}
        doneExpanded
        onToggleDone={() => undefined}
        onCloseDone={() => undefined}
        {...props}
      />,
    )
  })
}

function sectionSnapshot() {
  return {
    headings: Array.from(container.querySelectorAll('.section-heading')).map(
      (heading) =>
        Array.from(heading.children)
          .filter((child) => !child.matches('button'))
          .map((child) => child.textContent)
          .join(''),
    ),
    laterCount: container.querySelector('.later-section-count')?.textContent,
    visibleSessionIds: Array.from(
      container.querySelectorAll('.session-title'),
    ).map((title) => title.textContent),
    resultCount: container.querySelector('.inbox-filter-result-count')
      ?.textContent,
  }
}

function connectionSnapshot() {
  return {
    label: container.querySelector('.mobile-inbox-connection')?.textContent,
    dotClassName: container.querySelector('.mobile-inbox-connection-dot')
      ?.className,
  }
}

function foldedTodoCategorySnapshot() {
  return {
    needsActionCount: container.querySelector(
      '.needs-action-section .section-count',
    )?.textContent,
    inProgressTitles: Array.from(
      container.querySelectorAll('.in-progress-section .session-title'),
      (title) => title.textContent,
    ),
    inProgressTodoLaterCount: container.querySelector(
      '.mobile-in-progress-todo-later-count',
    )?.textContent,
    laterCount: container.querySelector('.later-section-count')?.textContent,
    laterTitles: Array.from(
      container.querySelectorAll('.later-session-list .session-title'),
      (title) => title.textContent,
    ),
    doneTitles: Array.from(
      container.querySelectorAll('.done-section .session-title'),
      (title) => title.textContent,
    ),
    todoLaterSectionExists: container.querySelector('.todo-later-row') !== null,
  }
}

function doneRowSnapshot() {
  return {
    text: container.querySelector('.mobile-done-toggle')?.textContent,
    expanded: container
      .querySelector('.mobile-done-toggle')
      ?.getAttribute('aria-expanded'),
    chevronHidden: container
      .querySelector('.mobile-done-row .mobile-collapse-chevron')
      ?.getAttribute('aria-hidden'),
  }
}

function appliedFilterSnapshot() {
  return {
    dialogOpen: container.querySelector('[role="dialog"]') !== null,
    chips: container.querySelector('.inbox-filter-chips')?.textContent,
    needsActionCount: container.querySelector(
      '.needs-action-section .section-count',
    )?.textContent,
    visibleTitles: Array.from(container.querySelectorAll('.session-title')).map(
      (title) => title.textContent,
    ),
  }
}

function submitFilterQuery(query: string) {
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (input === null) return
  const valueSetter = Reflect.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  if (valueSetter === undefined) return
  Reflect.apply(valueSetter, input, [query])
  input.dispatchEvent(new Event('input', { bubbles: true }))
  container
    .querySelector('form.inbox-filter-search-row')
    ?.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
}

function chipSnapshot() {
  return {
    chips: container.querySelector('.inbox-filter-chips')?.textContent,
    visibleTitles: Array.from(container.querySelectorAll('.session-title')).map(
      (title) => title.textContent,
    ),
  }
}

function dialogSnapshot() {
  return { isOpen: container.querySelector('[role="dialog"]') !== null }
}

function clickFilterOption(value: string) {
  const option = Array.from(
    container.querySelectorAll<HTMLLabelElement>('.inbox-filter-option'),
  ).find((label) => label.textContent.startsWith(value))
  option?.querySelector('input')?.click()
}

function slashInputSnapshot(event: KeyboardEvent) {
  return {
    defaultPrevented: event.defaultPrevented,
    ...dialogSnapshot(),
  }
}

function checkboxSnapshot() {
  return {
    query: container.querySelector<HTMLInputElement>(
      '[aria-label="Inbox filter query"]',
    )?.value,
    reviewChecked: Array.from(
      container.querySelectorAll<HTMLLabelElement>('.inbox-filter-option'),
    )
      .find((label) => label.textContent.startsWith('Review'))
      ?.querySelector<HTMLInputElement>('input')?.checked,
    ...dialogSnapshot(),
  }
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
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

it('filters each inbox section and updates the visible counts', () => {
  renderInbox({ filterQuery: 'project:web-app' })

  expect(sectionSnapshot()).toEqual({
    headings: ['Needs action1', 'In progress1', 'Done1'],
    laterCount: '1',
    visibleSessionIds: [
      'Check the retry policy',
      'Review the changes',
      'Add request metrics',
      'Document request limits',
    ],
    resultCount: '1 of 2',
  })
})

it('shows folded to-dos in their session categories without adding Needs action count', () => {
  const progressSession = sessions.at(2)
  if (progressSession === undefined) {
    throw new Error('Expected an in-progress session')
  }
  const laterSession = sessions.at(1)
  if (laterSession === undefined) {
    throw new Error('Expected a later session')
  }
  const doneSession = sessions.at(3)
  if (doneSession === undefined) {
    throw new Error('Expected a done session')
  }
  renderInbox({
    sessions: [
      ...sessions,
      {
        ...progressSession,
        id: 'folded-todo-session',
        category: 'in_progress',
        title: 'Choose a notification sound',
        todoLaterCount: 1,
        todoLaterTitle: 'Check the alert sound',
      },
      {
        ...laterSession,
        id: 'folded-later-session',
        todoLaterCount: 2,
        todoLaterTitle: 'Check delivery status',
      },
      {
        ...doneSession,
        id: 'folded-done-session',
        todoLaterCount: 1,
        todoLaterTitle: 'Review the alert sound',
      },
    ],
    isMobile: true,
    inProgressExpanded: true,
    laterExpanded: true,
    doneExpanded: true,
  })

  expect(foldedTodoCategorySnapshot()).toEqual({
    needsActionCount: '2',
    inProgressTitles: ['Add request metrics', 'Choose a notification sound'],
    inProgressTodoLaterCount: '1 to-do later',
    laterCount: '2',
    laterTitles: ['Review the changes', 'Review the changes'],
    doneTitles: ['Document request limits', 'Document request limits'],
    todoLaterSectionExists: false,
  })
})

it('labels a folded to-do in the desktop session preview', () => {
  const progressSession = sessions.at(2)
  if (progressSession === undefined) {
    throw new Error('Expected an in-progress session')
  }
  renderInbox({
    sessions: [
      ...sessions,
      {
        ...progressSession,
        id: 'folded-todo-session',
        todoLaterCount: 1,
        todoLaterTitle: 'Check the alert sound',
      },
    ],
    showAllInProgress: true,
  })

  expect(
    Array.from(
      container.querySelectorAll('.in-progress-section .session-preview'),
      (preview) => preview.textContent,
    ),
  ).toEqual(['To-do · later · Check the alert sound'])
})

it('labels the Starred session strip for assistive technology', () => {
  renderInbox({ isMobile: true })

  const snapshot = () => {
    const strip = container.querySelector('.mobile-today-strip')
    return {
      role: strip?.getAttribute('role'),
      label: strip?.getAttribute('aria-label'),
    }
  }
  expect(snapshot()).toEqual({ role: 'group', label: 'Starred sessions' })
})

it.each([
  ['desktop', false],
  ['phone', true],
] as const)('keeps quick actions out of %s Inbox rows', (_layout, isMobile) => {
  renderInbox({ isMobile })

  expect(container.querySelectorAll('.session-row-actions').length).toEqual(0)
})

it('places the mobile connection after the brand and before header actions', () => {
  renderInbox({ isMobile: true })

  const brand = container.querySelector('.mobile-inbox-brand')
  const dot = container.querySelector('.mobile-inbox-connection-dot')
  const headerSnapshot = () => ({
    children: Array.from(brand?.children ?? []).map((child) =>
      child.classList.contains('brand-mark')
        ? 'brand mark'
        : child.classList.contains('mobile-inbox-connection')
          ? 'connection status'
          : child.classList.contains('mobile-nav-actions')
            ? 'header actions'
            : child.textContent,
    ),
    connectionDotBorderRadius:
      dot === null ? null : getComputedStyle(dot).borderRadius,
  })

  expect(headerSnapshot()).toEqual({
    children: [
      'brand mark',
      'agent watch',
      'connection status',
      'header actions',
    ],
    connectionDotBorderRadius: '50%',
  })
})

it('keeps the iPhone inbox brand below the safe area', () => {
  container.className = 'agent-watch-shell'
  container.setAttribute('data-layout', 'mobile')
  renderInbox({ isMobile: true })

  const brand = container.querySelector('.mobile-inbox-brand')
  if (brand === null) throw new Error('Expected the mobile inbox brand')

  expect(getComputedStyle(brand).paddingTop).toEqual('8px')
})

it('shows the connection time while the iPhone inbox is connected', () => {
  renderInbox({ isMobile: true })

  expect(connectionSnapshot()).toEqual({
    label: 'Connected · 22:00',
    dotClassName: 'mobile-inbox-connection-dot',
  })
})

it.each([
  ['desktop', false],
  ['phone', true],
] as const)('shows the Later icon in the %s Inbox', (_layout, isMobile) => {
  renderInbox({ isMobile })

  expect(
    container.querySelector('.later-collapsed-row')?.firstElementChild
      ?.textContent,
  ).toEqual('◷ Later')
})

it('shows the last update time and error dot while the iPhone inbox is disconnected', () => {
  renderInbox({
    isMobile: true,
    error: 'The session feed disconnected.',
    connectionError: 'The session feed disconnected.',
  })

  expect(connectionSnapshot()).toEqual({
    label: 'Disconnected · Last updated 22:00',
    dotClassName: 'mobile-inbox-connection-dot is-disconnected',
  })
})

it('keeps a connected status when saved view storage fails', () => {
  renderInbox({
    isMobile: true,
    error: 'Unable to load saved inbox views.',
  })

  expect(connectionSnapshot()).toEqual({
    label: 'Connected · 22:00',
    dotClassName: 'mobile-inbox-connection-dot',
  })
})

it('shows the combined review count in the collapsed iPhone Done row', () => {
  const secondDoneSession: Session = {
    id: 'sample-done-secondary',
    category: 'done',
    isPaused: false,
    kind: null,
    title: 'Document another result',
    preview: null,
    elapsedSeconds: 60,
    unread: false,
    task: null,
    tasks: [],
    repo: 'sample-org/service',
    project: 'web-app',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
    unreviewedDecisionCount: 1,
  }
  renderInbox({
    isMobile: true,
    doneExpanded: false,
    sessions: [
      ...sessions.map((session) =>
        session.id === 'sample-done'
          ? { ...session, unreviewedDecisionCount: 1 }
          : session,
      ),
      secondDoneSession,
    ],
  })

  expect(doneRowSnapshot()).toEqual({
    text: 'Done2◇ 2 to review›',
    expanded: 'false',
    chevronHidden: 'true',
  })
})

it('shows the downward chevron for expanded iPhone Done rows with pending reviews', () => {
  renderInbox({
    isMobile: true,
    doneExpanded: true,
    sessions: sessions.map((session) =>
      session.id === 'sample-done'
        ? { ...session, unreviewedDecisionCount: 1 }
        : session,
    ),
  })

  expect(doneRowSnapshot()).toEqual({
    text: 'Done1◇ 1 to review⌄',
    expanded: 'true',
    chevronHidden: 'true',
  })
})

it('closes the Done sessions from the section action', () => {
  const onCloseDone = vi.fn()
  renderInbox({ onCloseDone })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.done-section .inbox-close-all')
      ?.click()
  })

  expect(onCloseDone.mock.calls).toEqual([
    [sessions.filter((s) => s.category === 'done')],
  ])
})

it('closes the Done sessions from the mobile collapse row', () => {
  const onCloseDone = vi.fn()
  renderInbox({ isMobile: true, onCloseDone })

  act(() => {
    container.querySelector<HTMLButtonElement>('.mobile-close-all')?.click()
  })

  expect(onCloseDone.mock.calls).toEqual([
    [sessions.filter((s) => s.category === 'done')],
  ])
})

it('closes all Done sessions even when filters hide them', () => {
  const onCloseDone = vi.fn()
  renderInbox({ filterQuery: 'project:missing', onCloseDone })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.done-section .inbox-close-all')
      ?.click()
  })

  expect(onCloseDone.mock.calls).toEqual([
    [sessions.filter((s) => s.category === 'done')],
  ])
})

it('opens the filter dialog from its button', () => {
  renderInbox()

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  expect(dialogSnapshot()).toEqual({ isOpen: true })
})

it('shows the iPhone filter as a full-screen dialog and applies its query', () => {
  const onFilterQueryChange = vi.fn()
  const onFilterDialogOpenChange = vi.fn()
  renderInbox({
    isMobile: true,
    filterQuery: 'project:web-app',
    filterDialogOpen: true,
    onFilterQueryChange,
    onFilterDialogOpenChange,
  })
  const dialog = container.querySelector('.inbox-filter-mobile-dialog')

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-mobile-footer button')
      ?.click()
  })

  const snapshot = () => ({
    presentation: container
      .querySelector('.mobile-sheet-overlay')
      ?.getAttribute('data-presentation'),
    dialogRole: dialog?.getAttribute('role'),
    appliedQuery: onFilterQueryChange.mock.calls,
    openChanges: onFilterDialogOpenChange.mock.calls,
  })
  expect(snapshot()).toEqual({
    presentation: 'fullscreen',
    dialogRole: 'dialog',
    appliedQuery: [['project:web-app']],
    openChanges: [[false]],
  })
})

it('keeps keyboard focus inside the iPhone filter dialog', () => {
  renderInbox({ isMobile: true })
  const openButton =
    container.querySelector<HTMLButtonElement>('.inbox-filter-open')
  const dialog = container.querySelector<HTMLElement>(
    '.inbox-filter-mobile-dialog',
  )
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  const lastButton = container.querySelector<HTMLButtonElement>(
    '.inbox-filter-mobile-footer button',
  )
  if (
    openButton === null ||
    dialog !== null ||
    input !== null ||
    lastButton !== null
  ) {
    throw new Error('Expected the iPhone filter to start closed')
  }

  act(() => {
    openButton.focus()
    openButton.click()
  })
  const activeDialog = container.querySelector<HTMLElement>(
    '.inbox-filter-mobile-dialog',
  )
  const activeInput = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  if (activeDialog === null || activeInput === null) {
    throw new Error('Expected the iPhone filter controls')
  }
  const inputFocusedOnOpen = document.activeElement === activeInput
  act(() => {
    openButton.focus()
  })
  const focusReturnedInsideDialog = activeDialog.contains(
    document.activeElement,
  )
  act(() => {
    activeDialog.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    inputFocusedOnOpen,
    focusReturnedInsideDialog,
    dialogClosed:
      container.querySelector('.inbox-filter-mobile-dialog') === null,
    focusRestored: document.activeElement === openButton,
  })
  expect(snapshot()).toEqual({
    inputFocusedOnOpen: true,
    focusReturnedInsideDialog: true,
    dialogClosed: true,
    focusRestored: true,
  })
})

it('opens the filter dialog with slash', () => {
  renderInbox()

  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  expect(dialogSnapshot()).toEqual({ isOpen: true })
})

it('leaves slash unhandled when the filter query input has focus', () => {
  renderInbox()
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Inbox filter query"]',
  )
  const slashEvent = new KeyboardEvent('keydown', {
    key: '/',
    bubbles: true,
    cancelable: true,
  })

  act(() => {
    input?.dispatchEvent(slashEvent)
  })

  expect(slashInputSnapshot(slashEvent)).toEqual({
    defaultPrevented: false,
    isOpen: true,
  })
})

it('closes the filter dialog with Escape', () => {
  renderInbox()
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  act(() => {
    container.querySelector('[aria-label="Inbox filter query"]')?.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  expect(dialogSnapshot()).toEqual({ isOpen: false })
})

it('applies the query when the dialog form is submitted', () => {
  renderInbox()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  act(() => {
    submitFilterQuery('project:worker')
  })

  expect(appliedFilterSnapshot()).toEqual({
    dialogOpen: false,
    chips: 'Project worker×',
    needsActionCount: '1',
    visibleTitles: ['Inspect worker output'],
  })
})

it('removes only the filter chip that was cleared', () => {
  renderInbox()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  act(() => {
    submitFilterQuery('project:web-app kind:review')
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Remove Project filter"]')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  expect(chipSnapshot()).toEqual({
    chips: 'Kind review×',
    visibleTitles: ['Review the changes'],
  })
})

it('clears all active inbox filters from the mobile filter bar', () => {
  const onFilterQueryChange = vi.fn()
  renderInbox({
    isMobile: true,
    filterQuery: 'project:web-app kind:review',
    onFilterQueryChange,
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.inbox-filter-clear')?.click()
  })

  expect(onFilterQueryChange.mock.calls).toEqual([['']])
})

it('shows swipe actions and runs Later or Close without opening the session', () => {
  const onSelect = vi.fn()
  const onLater = vi.fn()
  const onCloseSession = vi.fn()
  renderInbox({ isMobile: true, onSelect, onLater, onCloseSession })
  const row = container.querySelector<HTMLButtonElement>(
    '.needs-action-section [data-session-row]',
  )
  if (row === null) throw new Error('Expected the sample inbox row')
  const pointer = (type: string, x: number) =>
    row.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: x,
        clientY: 20,
      }),
    )

  act(() => {
    pointer('pointerdown', 260)
    pointer('pointermove', 120)
    pointer('pointerup', 120)
  })
  const swipeSnapshot = () => ({
    isSwiped: row
      .closest('.session-list-item')
      ?.classList.contains('is-swiped'),
    actions: Array.from(
      row
        .closest('.session-list-item')
        ?.querySelectorAll('button[aria-label]') ?? [],
      (button) => button.getAttribute('aria-label'),
    ),
    selectCalls: onSelect.mock.calls,
  })
  const initialSwipeState = swipeSnapshot()

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Later: Check the retry policy"]',
      )
      ?.click()
  })
  act(() => {
    pointer('pointerdown', 260)
    pointer('pointermove', 120)
    pointer('pointerup', 120)
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Close: Check the retry policy"]',
      )
      ?.click()
  })

  const snapshot = () => ({
    initialSwipeState,
    laterCalls: onLater.mock.calls,
    closeCalls: onCloseSession.mock.calls,
    selectCalls: onSelect.mock.calls,
  })
  expect(snapshot()).toEqual({
    initialSwipeState: {
      isSwiped: true,
      actions: [
        'Later: Check the retry policy',
        'Close: Check the retry policy',
      ],
      selectCalls: [],
    },
    laterCalls: [['sample-needs-action']],
    closeCalls: [['sample-needs-action']],
    selectCalls: [],
  })
})

it('reveals only Later for a done session', () => {
  renderInbox({
    isMobile: true,
    onLater: () => undefined,
    onCloseSession: () => undefined,
  })
  const row = container.querySelector<HTMLButtonElement>(
    '.done-section [data-session-row]',
  )
  if (row === null) throw new Error('Expected the sample done row')

  act(() => {
    for (const [type, x] of [
      ['pointerdown', 260],
      ['pointermove', 120],
      ['pointerup', 120],
    ] as const) {
      row.dispatchEvent(
        new PointerEvent(type, {
          bubbles: true,
          cancelable: true,
          pointerId: 1,
          pointerType: 'touch',
          clientX: x,
          clientY: 20,
        }),
      )
    }
  })

  const item = row.closest('.session-list-item')
  const snapshot = () => ({
    isSwiped: item?.classList.contains('is-swiped'),
    transform: row.style.transform,
    actions: Array.from(
      item?.querySelectorAll('button[aria-label]') ?? [],
      (button) => button.getAttribute('aria-label'),
    ),
  })
  expect(snapshot()).toEqual({
    isSwiped: true,
    transform: 'translateX(-88px)',
    actions: ['Later: Document request limits'],
  })
})

it('keeps swipe actions visible when vertical scrolling starts on a swiped row', () => {
  const onSelect = vi.fn()
  renderInbox({
    isMobile: true,
    onLater: () => undefined,
    onCloseSession: () => undefined,
    onSelect,
  })
  const row = container.querySelector<HTMLButtonElement>(
    '.needs-action-section [data-session-row]',
  )
  if (row === null) throw new Error('Expected the sample inbox row')
  const dispatchPointer = (type: string, x: number, y: number) =>
    row.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: x,
        clientY: y,
      }),
    )

  act(() => {
    dispatchPointer('pointerdown', 260, 20)
    dispatchPointer('pointermove', 120, 20)
    dispatchPointer('pointerup', 120, 20)
  })
  act(() => {
    dispatchPointer('pointerdown', 260, 20)
    dispatchPointer('pointermove', 260, 40)
  })
  act(() => {
    dispatchPointer('pointerup', 260, 40)
    row.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  const item = row.closest('.session-list-item')
  const snapshot = () => ({
    isSwiped: item?.classList.contains('is-swiped'),
    transform: row.style.transform,
    actions: Array.from(
      item?.querySelectorAll('button[aria-label]') ?? [],
      (button) => button.getAttribute('aria-label'),
    ),
    selectCalls: onSelect.mock.calls,
  })
  expect(snapshot()).toEqual({
    isSwiped: true,
    transform: 'translateX(-176px)',
    actions: ['Later: Check the retry policy', 'Close: Check the retry policy'],
    selectCalls: [],
  })
})

it('opens the mobile row actions sheet after a long press', () => {
  const onLater = vi.fn()
  const onCloseSession = vi.fn()
  renderInbox({ isMobile: true, onLater, onCloseSession })
  const row = container.querySelector<HTMLButtonElement>(
    '.needs-action-section [data-session-row]',
  )
  if (row === null) throw new Error('Expected the sample inbox row')
  vi.useFakeTimers()

  act(() => {
    row.dispatchEvent(
      new PointerEvent('pointerdown', {
        bubbles: true,
        cancelable: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: 100,
        clientY: 20,
      }),
    )
  })
  act(() => {
    vi.advanceTimersByTime(500)
  })

  const actionSheet = container.querySelector('[aria-label="Session actions"]')
  const actionSheetSnapshot = {
    open: actionSheet?.getAttribute('role'),
    focusInside: actionSheet?.contains(document.activeElement),
    actions: Array.from(
      actionSheet?.querySelectorAll('[role="menuitem"]') ?? [],
      (item) => item.textContent.trim(),
    ),
  }
  act(() => {
    actionSheet
      ?.querySelector<HTMLButtonElement>(
        '[role="menuitem"].session-row-actions-close',
      )
      ?.click()
  })
  const snapshot = () => ({
    actionSheetSnapshot,
    closeCalls: onCloseSession.mock.calls,
    laterCalls: onLater.mock.calls,
    sheetOpen:
      container.querySelector('[aria-label="Session actions"]') !== null,
  })
  expect(snapshot()).toEqual({
    actionSheetSnapshot: {
      open: 'dialog',
      focusInside: true,
      actions: ['Later›', 'Close session…'],
    },
    closeCalls: [['sample-needs-action']],
    laterCalls: [],
    sheetOpen: false,
  })
  vi.useRealTimers()
})

it('runs Later and dismisses the row actions sheet after a long press', () => {
  const onLater = vi.fn()
  renderInbox({ isMobile: true, onLater })
  const row = container.querySelector<HTMLButtonElement>(
    '.needs-action-section [data-session-row]',
  )
  if (row === null) throw new Error('Expected the sample inbox row')
  vi.useFakeTimers()

  act(() => {
    row.dispatchEvent(
      new PointerEvent('pointerdown', {
        bubbles: true,
        cancelable: true,
        pointerId: 1,
        pointerType: 'touch',
        clientX: 100,
        clientY: 20,
      }),
    )
    vi.advanceTimersByTime(500)
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Session actions"] [role="menuitem"]',
      )
      ?.click()
  })

  const snapshot = () => ({
    laterCalls: onLater.mock.calls,
    sheetOpen:
      container.querySelector('[aria-label="Session actions"]') !== null,
  })
  expect(snapshot()).toEqual({
    laterCalls: [['sample-needs-action']],
    sheetOpen: false,
  })
})

it('cancels a long press when another pointer touches the row', () => {
  renderInbox({ isMobile: true, onLater: () => undefined })
  const row = container.querySelector<HTMLButtonElement>(
    '.needs-action-section [data-session-row]',
  )
  if (row === null) throw new Error('Expected the sample inbox row')
  vi.useFakeTimers()

  act(() => {
    for (const pointerId of [1, 2]) {
      row.dispatchEvent(
        new PointerEvent('pointerdown', {
          bubbles: true,
          cancelable: true,
          pointerId,
          pointerType: 'touch',
          clientX: 100,
          clientY: 20,
        }),
      )
    }
    vi.advanceTimersByTime(500)
  })

  const snapshot = () => ({
    sheetOpen:
      container.querySelector('[aria-label="Session actions"]') !== null,
    transform: row.style.transform,
  })
  expect(snapshot()).toEqual({ sheetOpen: false, transform: 'translateX(0px)' })
})

it('shows the mobile Save view sheet and saves its entered name', () => {
  const onSaveView = vi.fn()
  renderInbox({
    isMobile: true,
    filterQuery: 'project:web-app kind:review',
    saveViewOpen: true,
    saveViewName: 'Service reviews',
    onSaveView,
  })
  const saveViewDialog = container.querySelector('[aria-label="Save view"]')

  act(() => {
    saveViewDialog
      ?.querySelector<HTMLButtonElement>('button[type="submit"]')
      ?.click()
  })

  const snapshot = () => ({
    role: saveViewDialog?.getAttribute('role'),
    summary: saveViewDialog?.querySelector('.inbox-save-view-summary')
      ?.textContent,
    saveCalls: onSaveView.mock.calls,
  })
  expect(snapshot()).toEqual({
    role: 'dialog',
    summary: 'Project web-app · Kind review',
    saveCalls: [['Service reviews']],
  })
})

it('toggles dialog options while retaining other filter fields', () => {
  renderInbox()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.inbox-filter-open')
      ?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  act(() => {
    clickFilterOption('web-app')
  })
  act(() => {
    Array.from(
      container.querySelectorAll<HTMLButtonElement>('.inbox-filter-field'),
    )
      .find((button) => button.textContent.includes('Kind'))
      ?.click()
  })
  act(() => {
    clickFilterOption('Review')
  })
  act(() => {
    clickFilterOption('Review')
  })

  expect(checkboxSnapshot()).toEqual({
    query: 'project:web-app',
    reviewChecked: false,
    isOpen: true,
  })
})
