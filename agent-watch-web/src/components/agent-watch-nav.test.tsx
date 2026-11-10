import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { AgentWatchNav } from '#components/agent-watch-nav'

let container: HTMLDivElement
let root: Root | null = null

function renderNav(
  workTypeNavGroups: ComponentProps<typeof AgentWatchNav>['workTypeNavGroups'],
  onSelectSession: (id: string) => void = () => undefined,
  savedViews: ComponentProps<typeof AgentWatchNav>['savedViews'] = [],
  selectedSavedViewId: string | null = null,
  onSelectSavedView: (id: string) => void = () => undefined,
  onDeleteSavedView: (id: string) => void = () => undefined,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <AgentWatchNav
        activeView="inbox"
        needsActionCount={0}
        savedViews={savedViews}
        selectedSavedViewId={selectedSavedViewId}
        workTypeNavGroups={workTypeNavGroups}
        onViewChange={() => undefined}
        onSelectSavedView={onSelectSavedView}
        onDeleteSavedView={onDeleteSavedView}
        todaySessions={[]}
        onSelectToday={() => undefined}
        onSelectSession={onSelectSession}
        onOpenPalette={() => undefined}
        inboxCollapsed={false}
        onToggleInbox={() => undefined}
      />,
    )
  })
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

it('renders the configured label and opens the selected session', () => {
  const onSelectSession = vi.fn<(id: string) => void>()
  renderNav(
    [
      {
        label: 'Guides',
        projects: [
          {
            name: 'sample project',
            status: 'Review',
            needsActionCount: 1,
            sessionId: 'sample-session',
          },
        ],
      },
    ],
    onSelectSession,
  )

  const button = container.querySelector<HTMLButtonElement>(
    '[aria-label="Open sample project Guides session in Inbox"]',
  )
  act(() => button?.click())

  const snapshot = () => ({
    headings: Array.from(
      container.querySelectorAll<HTMLElement>('.work-type-nav h2'),
      (heading) => heading.textContent,
    ),
    buttons: Array.from(
      container.querySelectorAll<HTMLButtonElement>('.work-type-project'),
      (element) => element.getAttribute('aria-label'),
    ),
    selectedSessions: onSelectSession.mock.calls,
  })

  expect(snapshot()).toEqual({
    headings: ['Guides'],
    buttons: ['Open sample project Guides session in Inbox'],
    selectedSessions: [['sample-session']],
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

it('selects the active saved view and shows its needs-action count color', () => {
  const onSelectSavedView = vi.fn<(id: string) => void>()
  renderNav(
    [],
    () => undefined,
    [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
    'reviews',
    onSelectSavedView,
  )

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
  renderNav(
    [],
    () => undefined,
    [
      {
        id: 'reviews',
        name: 'Sample reviews',
        query: 'project:sample-project kind:review',
        count: 3,
        countTone: 'needs-action',
      },
    ],
    null,
    () => undefined,
    onDeleteSavedView,
  )
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
