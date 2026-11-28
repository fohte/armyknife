import '#index.css'

import { act, type ComponentProps, useEffect, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

import {
  AsyncActionFeedbackProvider,
  useAsyncActionStarter,
} from '#async-action-feedback'
import { AgentWatchNav } from '#components/agent-watch-nav'
import { MobileNavDrawer } from '#components/mobile-nav-drawer'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null
let titleBarFixture: HTMLDivElement | null = null

const baseTodaySession: Session = {
  id: 'today-session',
  engine: 'claude',
  workType: 'development',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Review the retry policy',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'sample/repository',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderNav(
  workTypeNavGroups: ComponentProps<typeof AgentWatchNav>['workTypeNavGroups'],
  overrides: Partial<
    Omit<ComponentProps<typeof AgentWatchNav>, 'workTypeNavGroups'>
  > = {},
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <AgentWatchNav
        activeView="inbox"
        paletteShortcut={{
          modifier: 'ctrlKey',
          label: 'Ctrl K',
          ariaLabel: 'Control K',
        }}
        needsActionCount={0}
        savedViews={[]}
        selectedSavedViewId={null}
        workTypeNavGroups={workTypeNavGroups}
        onViewChange={() => undefined}
        onSelectSavedView={() => undefined}
        onDeleteSavedView={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        onSelectSession={() => undefined}
        onOpenPalette={() => undefined}
        inboxCollapsed={false}
        onToggleInbox={() => undefined}
        {...overrides}
      />,
    )
  })
}

function StartFailedAction() {
  const start = useAsyncActionStarter(() => undefined)

  useEffect(() => {
    start({
      kind: 'merge',
      sessionId: 'fixture-session',
      sessionTitle: 'Fixture session',
      run: () =>
        Promise.resolve({ status: 'mergeFailed', error: 'Fixture failure' }),
    })
  }, [start])

  return null
}

function MobileNavigationWithActivity({
  onCloseNavigation,
}: {
  onCloseNavigation: () => void
}) {
  const [open, setOpen] = useState(true)
  const closeNavigation = () => {
    onCloseNavigation()
    setOpen(false)
  }

  return (
    <MobileNavDrawer open={open} onClose={closeNavigation}>
      <AgentWatchNav
        activeView="inbox"
        isMobile
        paletteShortcut={{
          modifier: 'ctrlKey',
          label: 'Ctrl K',
          ariaLabel: 'Control K',
        }}
        needsActionCount={0}
        savedViews={[]}
        selectedSavedViewId={null}
        workTypeNavGroups={[]}
        onViewChange={() => undefined}
        onSelectSavedView={() => undefined}
        onDeleteSavedView={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        onSelectSession={() => undefined}
        onOpenPalette={() => undefined}
        onCloseNavigation={closeNavigation}
      />
    </MobileNavDrawer>
  )
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  document.documentElement.removeAttribute('data-agent-watch-desktop')
  titleBarFixture = null
  container = document.createElement('div')
  container.className = 'agent-watch-shell'
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  titleBarFixture?.remove()
  titleBarFixture = null
  document.documentElement.removeAttribute('data-agent-watch-desktop')
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('drags desktop headers while preserving browser and mobile header regions', () => {
  titleBarFixture = document.createElement('div')
  const fixture = titleBarFixture
  fixture.innerHTML = `
    <section class="inbox-view">
      <header class="view-header"><button>Close</button></header>
    </section>
    <section class="conversation-pane">
      <header class="conversation-header"><a href="#session">Session</a></header>
    </section>
    <section class="conversation-pane" data-mobile="true">
      <header class="conversation-header"><a href="#mobile-session">Mobile session</a></header>
    </section>
  `
  document.body.append(fixture)
  renderNav([])

  const brandRow = container.querySelector<HTMLElement>('.brand-row')
  const brandButton =
    container.querySelector<HTMLButtonElement>('.brand-row button')
  const inboxHeader = fixture.querySelector<HTMLElement>('.view-header')
  const inboxButton = fixture.querySelector<HTMLButtonElement>('button')
  const conversationHeader = fixture.querySelector<HTMLElement>(
    '.conversation-header',
  )
  const conversationLink = fixture.querySelector<HTMLAnchorElement>('a')
  const mobileConversationHeader = fixture.querySelector<HTMLElement>(
    '.conversation-pane[data-mobile="true"] .conversation-header',
  )
  const mobileConversationLink = fixture.querySelector<HTMLAnchorElement>(
    '.conversation-pane[data-mobile="true"] a',
  )
  const navigation = container.querySelector<HTMLElement>('.agent-watch-nav')
  const navigationScroll = container.querySelector<HTMLElement>(
    '.agent-watch-nav-scroll',
  )
  if (
    brandRow === null ||
    brandButton === null ||
    inboxHeader === null ||
    inboxButton === null ||
    conversationHeader === null ||
    conversationLink === null ||
    mobileConversationHeader === null ||
    mobileConversationLink === null ||
    navigation === null ||
    navigationScroll === null
  ) {
    throw new Error('Title-bar header fixture is incomplete')
  }

  navigation.style.height = '120px'
  navigationScroll.style.height = '80px'
  navigationScroll.style.maxHeight = '80px'
  navigationScroll.style.flex = 'none'
  const scrollFiller = document.createElement('div')
  scrollFiller.style.height = '320px'
  navigationScroll.append(scrollFiller)

  const region = (element: Element) =>
    getComputedStyle(element).getPropertyValue('-webkit-app-region')
  const measureRowScrollOffset = () => {
    const initialTop = brandRow.getBoundingClientRect().top
    navigationScroll.scrollTop = 64
    const scrollOffset = Math.round(
      brandRow.getBoundingClientRect().top - initialTop,
    )
    navigationScroll.scrollTop = 0
    return scrollOffset
  }
  const browserScrollOffset = measureRowScrollOffset()
  const snapshot = () => ({
    browserPadding: getComputedStyle(brandRow).paddingLeft,
    browserPosition: getComputedStyle(brandRow).position,
    browserScrollOffset,
    browserRegions: [
      region(brandRow),
      region(brandButton),
      region(inboxHeader),
      region(inboxButton),
      region(conversationHeader),
      region(conversationLink),
      region(mobileConversationHeader),
      region(mobileConversationLink),
    ],
  })
  const desktopScrollOffset = () => measureRowScrollOffset()
  const desktopSnapshot = () => ({
    desktopPadding: getComputedStyle(brandRow).paddingLeft,
    desktopPosition: getComputedStyle(brandRow).position,
    desktopScrollOffset: desktopScrollOffset(),
    desktopRegions: [
      region(brandRow),
      region(brandButton),
      region(inboxHeader),
      region(inboxButton),
      region(conversationHeader),
      region(conversationLink),
      region(mobileConversationHeader),
      region(mobileConversationLink),
    ],
  })

  expect(snapshot()).toEqual({
    browserPadding: '8px',
    browserPosition: 'static',
    browserScrollOffset: -64,
    browserRegions: [
      'none',
      'none',
      'none',
      'none',
      'none',
      'none',
      'none',
      'none',
    ],
  })

  document.documentElement.dataset['agentWatchDesktop'] = 'true'
  expect(desktopSnapshot()).toEqual({
    desktopPadding: '64px',
    desktopPosition: 'sticky',
    desktopScrollOffset: 0,
    desktopRegions: [
      'drag',
      'no-drag',
      'drag',
      'no-drag',
      'drag',
      'no-drag',
      'none',
      'none',
    ],
  })
})

it('renders the configured label and opens the selected session', () => {
  const onSelectSession = vi.fn<(id: string) => void>()
  renderNav(
    [
      {
        label: 'Guides',
        sessions: [
          {
            ...baseTodaySession,
            id: 'sample-session',
            title: 'Sample session',
          },
        ],
      },
    ],
    { onSelectSession },
  )

  const button = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open Sample session Guides session in Inbox"]',
  )
  act(() => button?.click())

  const snapshot = () => ({
    headings: Array.from(
      container.querySelectorAll<HTMLElement>('.work-type-nav h2'),
      (heading) => heading.textContent,
    ),
    buttons: Array.from(
      container.querySelectorAll<HTMLButtonElement>('.work-type-nav button'),
      (element) => element.getAttribute('aria-label'),
    ),
    selectedSessions: onSelectSession.mock.calls,
  })

  expect(snapshot()).toEqual({
    headings: ['Guides'],
    buttons: ['Open Sample session Guides session in Inbox'],
    selectedSessions: [['sample-session']],
  })
})

it('opens a new session from the desktop navigation', async () => {
  const onNewSession = vi.fn()
  renderNav([], { onNewSession })
  const newSessionButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="New session"]',
  )
  if (newSessionButton === null)
    throw new Error('New session button is missing')

  await act(async () => {
    await userEvent.click(newSessionButton)
  })

  expect(onNewSession.mock.calls).toEqual([[]])
})

it('places the new-session action before the palette shortcut with the designed style', () => {
  renderNav([])

  const newSessionButton = container.querySelector<HTMLButtonElement>(
    '.brand-row [aria-label="New session"]',
  )
  const paletteButton = container.querySelector<HTMLButtonElement>(
    '.brand-row .agent-watch-command-key',
  )
  const plusIcon = newSessionButton?.querySelector('svg')
  if (
    newSessionButton === null ||
    paletteButton === null ||
    plusIcon === null ||
    plusIcon === undefined
  ) {
    throw new Error('The new-session button and icon are missing')
  }

  const snapshot = () => ({
    actions: Array.from(
      container.querySelectorAll<HTMLButtonElement>('.brand-row button'),
      (button) => button.getAttribute('aria-label'),
    ),
    plusColor: getComputedStyle(newSessionButton).color,
    plusSize: getComputedStyle(plusIcon).width,
    newSessionMarginLeft:
      Number.parseFloat(getComputedStyle(newSessionButton).marginLeft) > 0
        ? 'auto'
        : '0px',
    paletteMarginLeft:
      Number.parseFloat(getComputedStyle(paletteButton).marginLeft) > 0
        ? 'auto'
        : '0px',
  })

  expect(snapshot()).toEqual({
    actions: ['New session', 'Jump to with Control K', 'Hide Inbox'],
    plusColor: 'rgb(212, 212, 216)',
    plusSize: '15px',
    newSessionMarginLeft: 'auto',
    paletteMarginLeft: '0px',
  })
})

it('matches the designed section spacing and removes compact-row dividers', () => {
  renderNav(
    [
      {
        label: 'PM',
        sessions: [baseTodaySession],
      },
    ],
    {
      savedViews: [
        {
          id: 'reviews',
          name: 'Example reviews',
          query: 'kind:review',
          count: 1,
          countTone: null,
        },
      ],
      todaySessions: [baseTodaySession],
      sessions: [
        {
          ...baseTodaySession,
          id: 'recent-session',
          lastPromptedAt: '2026-01-01T11:00:00.000Z',
        },
      ],
    },
  )

  const savedViews = container.querySelector<HTMLElement>('.saved-views-nav')
  const workType = container.querySelector<HTMLElement>('.work-type-nav')
  if (savedViews === null || workType === null) {
    throw new Error('The saved views and work type sections are missing')
  }

  const snapshot = () => ({
    savedViewsMarginTop: getComputedStyle(savedViews).marginTop,
    workTypeMarginTop: getComputedStyle(workType).marginTop,
    compactRowBorders: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.today-nav li, .recent-nav li, .work-type-nav li',
      ),
      (row) => getComputedStyle(row).borderBottomWidth,
    ),
  })

  expect(snapshot()).toEqual({
    savedViewsMarginTop: '0px',
    workTypeMarginTop: '12px',
    compactRowBorders: ['0px', '0px', '0px'],
  })
})

it('renders no work type sections without configured groups', () => {
  renderNav([])

  expect(
    Array.from(
      container.querySelectorAll<HTMLElement>('.work-type-nav'),
      (section) => section.getAttribute('aria-labelledby'),
    ),
  ).toEqual([])
})

it('uses the shared compact row without a starred marker', () => {
  renderNav([], {
    todaySessions: [baseTodaySession],
  })

  const snapshot = () => ({
    heading: container.querySelector('#today-nav-title')?.textContent,
    rows: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.today-nav .session-row-content',
      ),
      (row) => ({
        title: row.querySelector('.session-title')?.textContent,
        role: row
          .querySelector('.session-role-icon')
          ?.getAttribute('aria-label'),
        engine: row.querySelector('.session-engine')?.textContent,
        task: row.querySelector('.session-task')?.textContent,
        repo: row.querySelector('.session-repo-label')?.textContent,
      }),
    ),
    starMarkers: container.querySelectorAll('.today-star').length,
  })

  expect(snapshot()).toEqual({
    heading: 'Starred',
    rows: [
      {
        title: 'Review the retry policy',
        role: 'development',
        engine: 'claude',
        task: '— No task',
        repo: 'sample/repository',
      },
    ],
    starMarkers: 0,
  })
})

it('shows the Waiting status for a starred session in navigation', () => {
  renderNav([], {
    todaySessions: [
      {
        ...baseTodaySession,
        id: 'waiting-session',
        category: 'waiting',
        kind: null,
        waitingReason: 'Waiting for the checks to finish',
      },
    ],
  })

  const row = container.querySelector('.today-nav .session-row-content')
  const status = row?.querySelector('.session-row-status')
  const snapshot = () => ({
    title: row?.querySelector('.session-title')?.textContent,
    status: status?.getAttribute('aria-label'),
    icon: status?.querySelector('svg')?.classList.contains('lucide-hourglass'),
    reason: row?.querySelector('.session-preview')?.textContent ?? null,
  })

  expect(snapshot()).toEqual({
    title: 'Review the retry policy',
    status: 'Waiting',
    icon: true,
    reason: null,
  })
})

it('shows only the ten most recently prompted sessions in newest-first order', () => {
  const promptedSessions = Array.from({ length: 12 }, (_, index) => ({
    ...baseTodaySession,
    id: `recent-${String(index + 1)}`,
    title: `Prompt ${String(index + 1)}`,
    lastPromptedAt: new Date(Date.UTC(2026, 0, 1, 0, index)).toISOString(),
  }))
  renderNav([], {
    sessions: [
      ...promptedSessions,
      { ...baseTodaySession, id: 'not-prompted', lastPromptedAt: null },
    ],
  })

  expect(
    Array.from(
      container.querySelectorAll<HTMLElement>('.recent-nav .session-title'),
      (title) => title.textContent,
    ),
  ).toEqual([
    'Prompt 12',
    'Prompt 11',
    'Prompt 10',
    'Prompt 9',
    'Prompt 8',
    'Prompt 7',
    'Prompt 6',
    'Prompt 5',
    'Prompt 4',
    'Prompt 3',
  ])
})

it('opens the selected session from Recent', () => {
  const onSelectSession = vi.fn<(id: string) => void>()
  renderNav([], {
    sessions: [
      {
        ...baseTodaySession,
        id: 'recent-session',
        lastPromptedAt: '2026-01-01T00:00:00Z',
      },
    ],
    onSelectSession,
  })

  const button = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open Review the retry policy recent session"]',
  )
  act(() => button?.click())

  expect(onSelectSession.mock.calls).toEqual([['recent-session']])
})

it('shows the Recent section in the mobile navigation', () => {
  renderNav([], {
    isMobile: true,
    sessions: [
      {
        ...baseTodaySession,
        lastPromptedAt: '2026-01-01T00:00:00Z',
      },
    ],
  })

  const snapshot = () => ({
    title: container.querySelector('#recent-nav-title')?.textContent,
    sessions: Array.from(
      container.querySelectorAll('.recent-nav .session-title'),
      (title) => title.textContent,
    ),
  })

  expect(snapshot()).toEqual({
    title: 'Recent',
    sessions: ['Review the retry policy'],
  })
})

it.each([
  {
    platform: 'macOS',
    paletteShortcut: {
      modifier: 'metaKey',
      label: '⌘K',
      ariaLabel: 'Command K',
    },
    label: '⌘K',
    ariaLabel: 'Jump to with Command K',
  },
  {
    platform: 'other platforms',
    paletteShortcut: {
      modifier: 'ctrlKey',
      label: 'Ctrl K',
      ariaLabel: 'Control K',
    },
    label: 'Ctrl K',
    ariaLabel: 'Jump to with Control K',
  },
] as const)(
  'shows the $label shortcut on $platform',
  ({ paletteShortcut, label, ariaLabel }) => {
    renderNav([], { paletteShortcut })

    const shortcut = container.querySelector<HTMLButtonElement>(
      '.agent-watch-command-key',
    )
    const snapshot = () => ({
      label: shortcut?.textContent,
      ariaLabel: shortcut?.getAttribute('aria-label'),
    })
    expect(snapshot()).toEqual({ label, ariaLabel })
  },
)

it('opens the command palette when its search icon is clicked', () => {
  let paletteOpened = false
  renderNav([], {
    onOpenPalette: () => {
      paletteOpened = true
    },
  })

  const searchIcon = container.querySelector('.agent-watch-command-key svg')
  act(() => {
    searchIcon?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  expect(paletteOpened).toBe(true)
})

it('selects the active saved view and shows its needs-action count color', () => {
  const onSelectSavedView = vi.fn<(id: string) => void>()
  renderNav([], {
    savedViews: [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
    selectedSavedViewId: 'reviews',
    onSelectSavedView,
  })

  act(() => {
    container.querySelector<HTMLButtonElement>('.saved-view-link')?.click()
  })

  const snapshot = () => ({
    heading: container.querySelector('#saved-views-title')?.textContent,
    selected: container
      .querySelector<HTMLButtonElement>('.saved-view-link')
      ?.getAttribute('aria-pressed'),
    countTone: container.querySelector('.saved-view-count')?.className,
    selectedIds: onSelectSavedView.mock.calls,
  })

  expect(snapshot()).toEqual({
    heading: 'VIEWS',
    selected: 'true',
    countTone: 'saved-view-count is-needs-action',
    selectedIds: [['reviews']],
  })
})

it('deletes a saved view from the navigation', () => {
  const onDeleteSavedView = vi.fn<(id: string) => void>()
  renderNav([], {
    savedViews: [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
    onDeleteSavedView,
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Delete Sample reviews view"]',
      )
      ?.click()
  })

  const snapshot = () => onDeleteSavedView.mock.calls

  expect(snapshot()).toEqual([['reviews']])
})

it('deletes a saved view through its hover-visible control', async () => {
  const onDeleteSavedView = vi.fn<(id: string) => void>()
  renderNav([], {
    savedViews: [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
    onDeleteSavedView,
  })

  const item = container.querySelector<HTMLElement>('.saved-view-item')
  const deleteButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Delete Sample reviews view"]',
  )
  if (item === null || deleteButton === null) {
    throw new Error('Saved view controls were not rendered')
  }

  await userEvent.hover(item)
  await userEvent.click(deleteButton)

  expect(onDeleteSavedView.mock.calls).toEqual([['reviews']])
})

it('keeps the hidden delete button available to keyboard focus', () => {
  renderNav([], {
    savedViews: [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
  })

  const deleteButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Delete Sample reviews view"]',
  )
  const count = container.querySelector('.saved-view-count')
  if (deleteButton === null || count === null) {
    throw new Error('Saved view controls were not rendered')
  }
  const hidden = {
    opacity: window.getComputedStyle(deleteButton).opacity,
    pointerEvents: window.getComputedStyle(deleteButton).pointerEvents,
    tabIndex: deleteButton.tabIndex,
  }

  act(() => {
    deleteButton.focus()
  })

  const focused = {
    opacity: window.getComputedStyle(deleteButton).opacity,
    pointerEvents: window.getComputedStyle(deleteButton).pointerEvents,
    countOpacity: window.getComputedStyle(count).opacity,
  }

  const snapshot = () => ({ hidden, focused })

  expect(snapshot()).toEqual({
    hidden: { opacity: '0', pointerEvents: 'none', tabIndex: 0 },
    focused: { opacity: '1', pointerEvents: 'auto', countOpacity: '0' },
  })
})

it('hides the saved views section when no views are saved', () => {
  renderNav([])

  expect(container.querySelector('#saved-views-title')).toBeNull()
})

it('toggles the Inbox from the navigation and marks the collapsed state', () => {
  const onToggleInbox = vi.fn<() => void>()
  root = createRoot(container)
  act(() => {
    root?.render(
      <AgentWatchNav
        activeView="inbox"
        paletteShortcut={{
          modifier: 'ctrlKey',
          label: 'Ctrl K',
          ariaLabel: 'Control K',
        }}
        needsActionCount={0}
        savedViews={[]}
        selectedSavedViewId={null}
        workTypeNavGroups={[]}
        onViewChange={() => undefined}
        onSelectSavedView={() => undefined}
        onDeleteSavedView={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        onSelectSession={() => undefined}
        onOpenPalette={() => undefined}
        inboxCollapsed
        onToggleInbox={onToggleInbox}
      />,
    )
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Show Inbox"]')
      ?.click()
  })

  const snapshot = () => ({
    pressed: container
      .querySelector<HTMLButtonElement>('[aria-label="Show Inbox"]')
      ?.getAttribute('aria-pressed'),
    calls: onToggleInbox.mock.calls,
  })
  expect(snapshot()).toEqual({ pressed: 'true', calls: [[]] })
})

it('shows the failed Activity entry above quota in the mobile navigation', () => {
  renderNav([], {
    isMobile: true,
    activityCount: 2,
    activityFailed: true,
  })

  const inboxButton = container.querySelector<HTMLButtonElement>(
    '.view-switch-inbox-button',
  )
  const activityButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-button',
  )
  const activityEntry = container.querySelector<HTMLElement>(
    '.agent-watch-nav-activity',
  )
  const nav = container.querySelector('nav')
  if (
    inboxButton === null ||
    activityButton === null ||
    activityEntry === null ||
    nav === null
  ) {
    throw new Error('Mobile navigation Activity state is missing')
  }

  const snapshot = () => ({
    inboxLabel: inboxButton.getAttribute('aria-label'),
    inboxFailureIndicator:
      inboxButton.querySelector('.view-switch-inbox-failure') !== null,
    activityLabel: activityButton.getAttribute('aria-label'),
    searchIconVisible: container.querySelector('.agent-watch-command') !== null,
    navigationFooter: Array.from(nav.children, (element) => element.className),
    activityButtonClass: activityButton.className,
    activityEntryFlex: getComputedStyle(activityEntry).flex,
    activityEntryMargin: getComputedStyle(activityEntry).margin,
  })
  expect(snapshot()).toEqual({
    inboxLabel: 'Inbox, Activity has failed actions',
    inboxFailureIndicator: true,
    activityLabel: 'Activity, failed actions',
    searchIconVisible: false,
    navigationFooter: [
      'agent-watch-nav-scroll',
      'agent-watch-nav-activity',
      'quota-panel',
    ],
    activityButtonClass: 'async-action-activity-button is-drawer',
    activityEntryFlex: '0 0 auto',
    activityEntryMargin: '8px 0px 4px',
  })
})

it('fills the phone drawer and pins Activity and quota below its scroll list', async () => {
  await page.viewport(390, 844)
  container.style.display = 'flex'
  container.style.width = '320px'
  container.style.height = '844px'
  container.style.overflow = 'hidden'
  renderNav([], {
    isMobile: true,
    todaySessions: Array.from({ length: 24 }, (_, index) => ({
      ...baseTodaySession,
      id: `starred-${String(index)}`,
      title: `Starred session ${String(index)}`,
    })),
    activityCount: 3,
  })

  const nav = container.querySelector<HTMLElement>('.agent-watch-nav')
  const scroll = container.querySelector<HTMLElement>('.agent-watch-nav-scroll')
  const activity = container.querySelector<HTMLElement>(
    '.agent-watch-nav-activity',
  )
  const quota = container.querySelector<HTMLElement>('.quota-panel')
  if (nav === null || scroll === null || activity === null || quota === null) {
    throw new Error('The phone drawer footer is missing')
  }

  const snapshot = () => {
    const navRect = nav.getBoundingClientRect()
    const scrollRect = scroll.getBoundingClientRect()
    const activityRect = activity.getBoundingClientRect()
    const quotaRect = quota.getBoundingClientRect()
    return {
      navHeight: navRect.height,
      scrollHasOverflow: scroll.scrollHeight > scroll.clientHeight,
      scrollToActivityGap: activityRect.top - scrollRect.bottom,
      activityToQuotaGap: quotaRect.top - activityRect.bottom,
      quotaBottomGap: navRect.bottom - quotaRect.bottom,
    }
  }

  expect(snapshot()).toEqual({
    navHeight: 844,
    scrollHasOverflow: true,
    scrollToActivityGap: 8,
    activityToQuotaGap: 4,
    quotaBottomGap: 0,
  })
})

it('opens desktop Activity beside its row below the navigation list', async () => {
  await page.viewport(1440, 900)
  container.style.display = 'flex'
  container.style.width = '240px'
  container.style.height = '900px'
  container.style.overflow = 'hidden'
  root = createRoot(container)
  await act(async () => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <AgentWatchNav
          activeView="inbox"
          paletteShortcut={{
            modifier: 'metaKey',
            label: '⌘K',
            ariaLabel: 'Command K',
          }}
          needsActionCount={0}
          savedViews={[]}
          selectedSavedViewId={null}
          workTypeNavGroups={[]}
          onViewChange={() => undefined}
          onSelectSavedView={() => undefined}
          onDeleteSavedView={() => undefined}
          todaySessions={[]}
          onSelectToday={() => undefined}
          onSelectSession={() => undefined}
          onOpenPalette={() => undefined}
        />
      </AsyncActionFeedbackProvider>,
    )
    return Promise.resolve()
  })

  const nav = container.querySelector<HTMLElement>('.agent-watch-nav')
  const scroll = container.querySelector<HTMLElement>('.agent-watch-nav-scroll')
  const activity = container.querySelector<HTMLElement>(
    '.agent-watch-nav-activity',
  )
  const activityButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-button',
  )
  const quota = container.querySelector<HTMLElement>('.quota-panel')
  if (
    nav === null ||
    scroll === null ||
    activity === null ||
    activityButton === null ||
    quota === null
  ) {
    throw new Error('Desktop navigation Activity is missing')
  }

  quota.style.height = '200px'
  const anchor = activityButton.getBoundingClientRect()
  await act(async () => {
    await userEvent.click(activityButton)
  })

  const dialog = container.querySelector<HTMLElement>('.async-action-activity')
  const backdrop = container.querySelector<HTMLElement>(
    '.async-action-activity-backdrop',
  )
  if (dialog === null || backdrop === null) {
    throw new Error('The desktop Activity dialog is missing')
  }

  const snapshot = () => {
    const navRect = nav.getBoundingClientRect()
    const scrollRect = scroll.getBoundingClientRect()
    const activityRect = activity.getBoundingClientRect()
    const dialogRect = dialog.getBoundingClientRect()
    const quotaRect = quota.getBoundingClientRect()
    return {
      navigationFooter: Array.from(
        nav.children,
        (element) => element.className,
      ),
      activityButtonClass: activityButton.className,
      scrollToActivityGap: activityRect.top - scrollRect.bottom,
      activityToQuotaGap: quotaRect.top - activityRect.bottom,
      quotaBottomGap: navRect.bottom - quotaRect.bottom,
      dialogLeftGap: dialogRect.left - anchor.right,
      dialogToActivityBottomGap: dialogRect.bottom - anchor.bottom,
      backdropColor: getComputedStyle(backdrop).backgroundColor,
    }
  }
  expect(snapshot()).toEqual({
    navigationFooter: [
      'agent-watch-nav-scroll',
      'agent-watch-nav-activity',
      'quota-panel',
    ],
    activityButtonClass: 'async-action-activity-button is-empty is-open',
    scrollToActivityGap: 8,
    activityToQuotaGap: 4,
    quotaBottomGap: 0,
    dialogLeftGap: 16,
    dialogToActivityBottomGap: 0,
    backdropColor: 'rgba(0, 0, 0, 0)',
  })
})

it('marks Inbox when an async action in Activity fails', async () => {
  await page.viewport(390, 844)
  const onCloseNavigation = vi.fn()
  root = createRoot(container)
  await act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <StartFailedAction />
        <MobileNavigationWithActivity onCloseNavigation={onCloseNavigation} />
      </AsyncActionFeedbackProvider>,
    )
    return Promise.resolve()
  })

  const inboxButton = container.querySelector<HTMLButtonElement>(
    '.view-switch-inbox-button',
  )
  const activityButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-button',
  )
  const snapshot = () => ({
    inboxLabel: inboxButton?.getAttribute('aria-label'),
    inboxFailureIndicator:
      inboxButton !== null &&
      inboxButton.querySelector('.view-switch-inbox-failure') !== null,
    activityLabel: activityButton?.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    inboxLabel: 'Inbox, Activity has failed actions',
    inboxFailureIndicator: true,
    activityLabel: 'Activity, failed actions',
  })

  if (activityButton === null) {
    throw new Error('The Activity button is missing')
  }
  await act(async () => {
    await userEvent.click(activityButton)
  })

  const openSnapshot = () => ({
    closeNavigationCalls: onCloseNavigation.mock.calls,
    activityDialogVisible:
      container.querySelector('[role="dialog"][aria-label="Activity"]') !==
      null,
    navigationDialogOpen:
      container.querySelector<HTMLDialogElement>('.mobile-nav-drawer')?.open ??
      false,
  })
  expect(openSnapshot()).toEqual({
    closeNavigationCalls: [[]],
    activityDialogVisible: true,
    navigationDialogOpen: false,
  })

  const activityDialog = container.querySelector<HTMLElement>(
    '.async-action-activity',
  )
  const backdrop = container.querySelector<HTMLElement>(
    '.async-action-activity-backdrop',
  )
  if (activityDialog === null || backdrop === null) {
    throw new Error('The phone Activity sheet is missing')
  }

  const sheetSnapshot = () => ({
    dialogClass: activityDialog.className,
    dialogPosition: getComputedStyle(activityDialog).position,
    dialogWidth: activityDialog.getBoundingClientRect().width,
    dialogBottomGap:
      window.innerHeight - activityDialog.getBoundingClientRect().bottom,
    backdropClass: backdrop.className,
    backdropColor: getComputedStyle(backdrop).backgroundColor,
  })
  expect(sheetSnapshot()).toEqual({
    dialogClass: 'async-action-activity is-anchored',
    dialogPosition: 'static',
    dialogWidth: 390,
    dialogBottomGap: 0,
    backdropClass: 'async-action-activity-backdrop is-anchored',
    backdropColor: 'rgba(0, 0, 0, 0.55)',
  })
})

it('opens Activity as a sheet throughout the mobile layout breakpoint', async () => {
  await page.viewport(710, 844)
  const onCloseNavigation = vi.fn()
  root = createRoot(container)
  await act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <MobileNavigationWithActivity onCloseNavigation={onCloseNavigation} />
      </AsyncActionFeedbackProvider>,
    )
    return Promise.resolve()
  })

  const activityButton = container.querySelector<HTMLButtonElement>(
    '.async-action-activity-button',
  )
  if (activityButton === null) {
    throw new Error('The Activity button is missing')
  }
  await act(async () => {
    await userEvent.click(activityButton)
  })

  const dialog = container.querySelector<HTMLElement>('.async-action-activity')
  const backdrop = container.querySelector<HTMLElement>(
    '.async-action-activity-backdrop',
  )
  if (dialog === null || backdrop === null) {
    throw new Error('The Activity sheet is missing')
  }

  const dialogRect = dialog.getBoundingClientRect()
  const snapshot = () => ({
    dialogPosition: getComputedStyle(dialog).position,
    dialogWidth: dialogRect.width,
    dialogRightGap: window.innerWidth - dialogRect.right,
    dialogBottomGap: window.innerHeight - dialogRect.bottom,
    backdropColor: getComputedStyle(backdrop).backgroundColor,
  })
  expect(snapshot()).toEqual({
    dialogPosition: 'static',
    dialogWidth: 710,
    dialogRightGap: 0,
    dialogBottomGap: 0,
    backdropColor: 'rgba(0, 0, 0, 0.55)',
  })
})
