import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { AgentWatchNav } from '#components/agent-watch-nav'

let container: HTMLDivElement
let root: Root | null = null

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
    { onSelectSession },
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
