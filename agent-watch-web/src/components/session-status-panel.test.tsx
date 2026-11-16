import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  SessionStatusPanel,
  type SessionStatusPanelTab,
} from '#components/session-status-panel'

let container: HTMLDivElement
let root: Root | null = null

function renderPanel(
  activeTabId: string | null,
  onTabChange = vi.fn(),
  onClose = vi.fn(),
  runningCount = 0,
  tabs?: readonly SessionStatusPanelTab[],
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={
          tabs ?? [
            {
              id: 'running',
              label: 'Running',
              count: runningCount,
              content: 'Running',
            },
            { id: 'crit', label: 'crit', count: 1, content: 'Crit content' },
          ]
        }
        activeTabId={activeTabId}
        onTabChange={onTabChange}
        onClose={onClose}
      />,
    )
  })
  return onTabChange
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

it('hides tabs with no items and renders the active tab content', () => {
  renderPanel('crit')

  const snapshot = () => ({
    tabs: Array.from(container.querySelectorAll('[role="tab"]'), (tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
    content: container.querySelector('[role="tabpanel"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    tabs: [{ label: 'crit', selected: 'true' }],
    content: 'Crit content',
  })
})

it('reports the selected tab when a visible tab is clicked', () => {
  const onTabChange = renderPanel('crit')
  act(() => {
    container.querySelector<HTMLButtonElement>('[role="tab"]')?.click()
  })

  expect(onTabChange.mock.calls).toEqual([['crit']])
})

it('keeps overflowing tabs selectable and scrolls the active tab beside the close button', () => {
  const tabs = Array.from({ length: 8 }, (_, index) => ({
    id: `view-${String(index + 1)}`,
    label: `Integration view ${String(index + 1)} with a long label`,
    count: 1,
    content: `View ${String(index + 1)}`,
  }))
  const onTabChange = vi.fn()
  container.style.width = '320px'
  container.style.height = '480px'
  renderPanel('view-1', onTabChange, undefined, 0, tabs)

  const tabList = container.querySelector<HTMLElement>('[role="tablist"]')
  const lastTab = container.querySelector<HTMLButtonElement>(
    '#session-status-tab-view-8',
  )
  const closeButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Close session status panel"]',
  )
  const header = container.querySelector<HTMLElement>(
    '.session-status-panel-tabs',
  )

  act(() => lastTab?.click())
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={tabs}
        activeTabId="view-8"
        onTabChange={onTabChange}
        onClose={vi.fn()}
      />,
    )
  })

  const activeTabBounds = lastTab?.getBoundingClientRect()
  const tabListBounds = tabList?.getBoundingClientRect()
  const closeButtonBounds = closeButton?.getBoundingClientRect()
  const headerBounds = header?.getBoundingClientRect()
  const snapshot = () => ({
    tabCount: container.querySelectorAll('[role="tab"]').length,
    lastTabSelectable: onTabChange.mock.calls,
    tabListOverflows: (tabList?.scrollWidth ?? 0) > (tabList?.clientWidth ?? 0),
    activeTabVisible:
      activeTabBounds !== undefined &&
      tabListBounds !== undefined &&
      activeTabBounds.left >= tabListBounds.left &&
      activeTabBounds.right <= tabListBounds.right + 1,
    closeButtonVisible:
      closeButtonBounds !== undefined &&
      tabListBounds !== undefined &&
      headerBounds !== undefined &&
      closeButtonBounds.left >= tabListBounds.right &&
      closeButtonBounds.right <= headerBounds.right,
  })
  expect(snapshot()).toEqual({
    tabCount: 8,
    lastTabSelectable: [['view-8']],
    tabListOverflows: true,
    activeTabVisible: true,
    closeButtonVisible: true,
  })
})

it('shows the count for a single running item', () => {
  renderPanel('running', undefined, undefined, 1)

  const snapshot = Array.from(
    container.querySelectorAll('[role="tab"]'),
    (tab) => tab.textContent,
  )
  expect(snapshot).toEqual(['Running1', 'crit'])
})

it('keeps a related tree tab visible and colors pending descendants', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={[
          {
            id: 'tree',
            label: 'Tree',
            count: 1,
            visible: true,
            showCount: true,
            countTone: 'warning',
            content: 'Tree content',
          },
        ]}
        activeTabId="tree"
        onTabChange={vi.fn()}
        onClose={vi.fn()}
      />,
    )
  })

  const snapshot = () => ({
    tab: container.querySelector('[role="tab"]')?.textContent,
    countClass: container.querySelector('.session-status-panel-count')
      ?.className,
    content: container.querySelector('[role="tabpanel"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    tab: 'Tree1',
    countClass:
      'session-status-panel-count session-status-panel-count--warning',
    content: 'Tree content',
  })
})

it('closes the status panel from its tab bar', () => {
  const onClose = vi.fn()
  renderPanel('crit', undefined, onClose)
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Close session status panel"]',
      )
      ?.click()
  })

  expect(onClose.mock.calls).toEqual([[]])
})
