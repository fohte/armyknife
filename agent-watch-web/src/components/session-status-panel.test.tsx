import '#index.css'

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

function getTabLayoutSnapshot(activeTabId: string) {
  const tabList = container.querySelector<HTMLElement>('[role="tablist"]')
  const activeTab = container.querySelector<HTMLButtonElement>(
    `#session-status-tab-${activeTabId}`,
  )
  const closeButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Close session status panel"]',
  )
  const header = container.querySelector<HTMLElement>(
    '.session-status-panel-tabs',
  )
  const activeTabBounds = activeTab?.getBoundingClientRect()
  const tabListBounds = tabList?.getBoundingClientRect()
  const closeButtonBounds = closeButton?.getBoundingClientRect()
  const headerBounds = header?.getBoundingClientRect()
  const scrollLeft = tabList?.scrollLeft ?? 0
  const startPadding = Number.parseFloat(
    tabList === null ? '0' : getComputedStyle(tabList).paddingLeft,
  )

  return {
    hasScrolledRight: scrollLeft > startPadding,
    isAtStart: scrollLeft <= startPadding,
    fadesLeft: tabList?.parentElement?.classList.contains(
      'has-hidden-tabs-left',
    ),
    fadesRight: tabList?.parentElement?.classList.contains(
      'has-hidden-tabs-right',
    ),
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
  }
}

function getTabBarLayoutSnapshot() {
  const bar = container.querySelector<HTMLElement>('.session-status-panel-tabs')
  const tabList = container.querySelector<HTMLElement>('[role="tablist"]')
  const activeTab = container.querySelector<HTMLElement>(
    '[role="tab"][aria-selected="true"]',
  )
  const closeButton = container.querySelector<HTMLElement>(
    '.session-status-panel-close',
  )
  const barBounds = bar?.getBoundingClientRect()
  const activeTabBounds = activeTab?.getBoundingClientRect()
  const closeButtonBounds = closeButton?.getBoundingClientRect()
  const round = (value: number) => Math.round(value * 100) / 100

  return {
    barHeight: barBounds?.height,
    tabListHeight: tabList?.getBoundingClientRect().height,
    activeTabHeight: activeTabBounds?.height,
    activeTabBottomGap:
      barBounds === undefined || activeTabBounds === undefined
        ? null
        : round(barBounds.bottom - activeTabBounds.bottom),
    labelCountGap: activeTab === null ? null : getComputedStyle(activeTab).gap,
    closeTopInset:
      barBounds === undefined || closeButtonBounds === undefined
        ? null
        : round(closeButtonBounds.top - barBounds.top),
    closeRightInset:
      barBounds === undefined || closeButtonBounds === undefined
        ? null
        : round(barBounds.right - closeButtonBounds.right),
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
  act(() => {
    if (tabList !== null) tabList.scrollLeft = 300
    tabList?.dispatchEvent(new Event('scroll'))
    root?.render(
      <SessionStatusPanel
        tabs={tabs.map((tab) => ({ ...tab }))}
        activeTabId="view-1"
        onTabChange={onTabChange}
        onClose={vi.fn()}
      />,
    )
  })
  const userScrollPreserved = tabList?.scrollLeft === 300
  const manualScrollFades = {
    left: tabList?.parentElement?.classList.contains('has-hidden-tabs-left'),
    right: tabList?.parentElement?.classList.contains('has-hidden-tabs-right'),
  }

  const lastTab = container.querySelector<HTMLButtonElement>(
    '#session-status-tab-view-8',
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

  const rightwardScroll = getTabLayoutSnapshot('view-8')
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={tabs}
        activeTabId="view-1"
        onTabChange={onTabChange}
        onClose={vi.fn()}
      />,
    )
  })
  const leftwardScroll = getTabLayoutSnapshot('view-1')
  const snapshot = () => ({
    tabCount: container.querySelectorAll('[role="tab"]').length,
    lastTabSelectable: onTabChange.mock.calls,
    userScrollPreserved,
    manualScrollFades,
    tabListOverflows:
      (container.querySelector<HTMLElement>('[role="tablist"]')?.scrollWidth ??
        0) >
      (container.querySelector<HTMLElement>('[role="tablist"]')?.clientWidth ??
        0),
    rightwardScroll,
    leftwardScroll,
  })
  expect(snapshot()).toEqual({
    tabCount: 8,
    lastTabSelectable: [['view-8']],
    userScrollPreserved: true,
    manualScrollFades: { left: true, right: true },
    tabListOverflows: true,
    rightwardScroll: {
      hasScrolledRight: true,
      isAtStart: false,
      fadesLeft: true,
      fadesRight: false,
      activeTabVisible: true,
      closeButtonVisible: true,
    },
    leftwardScroll: {
      hasScrolledRight: false,
      isAtStart: true,
      fadesLeft: false,
      fadesRight: true,
      activeTabVisible: true,
      closeButtonVisible: true,
    },
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

it('aligns the status tabs with the full bar on desktop and phone', async () => {
  const tabs = [
    { id: 'overview', label: 'Overview', count: 3, content: 'Overview' },
    { id: 'updates', label: 'Updates', count: 5, content: 'Updates' },
  ]
  root = createRoot(container)

  const renderAndSettle = async (size: 'desktop' | 'phone') => {
    await act(async () => {
      container.style.width = size === 'desktop' ? '640px' : '390px'
      root?.render(
        <SessionStatusPanel
          tabs={tabs}
          activeTabId="updates"
          onTabChange={vi.fn()}
          onClose={vi.fn()}
          size={size}
        />,
      )
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            resolve()
          })
        })
      })
    })
  }

  await renderAndSettle('desktop')
  const desktopSnapshot = getTabBarLayoutSnapshot()
  await renderAndSettle('phone')
  const phoneSnapshot = getTabBarLayoutSnapshot()
  const snapshot = () => ({ desktop: desktopSnapshot, phone: phoneSnapshot })
  expect(snapshot()).toEqual({
    desktop: {
      barHeight: 48,
      tabListHeight: 47,
      activeTabHeight: 47,
      activeTabBottomGap: 1,
      labelCountGap: '6px',
      closeTopInset: 9.5,
      closeRightInset: 10,
    },
    phone: {
      barHeight: 45,
      tabListHeight: 44,
      activeTabHeight: 44,
      activeTabBottomGap: 1,
      labelCountGap: '6px',
      closeTopInset: null,
      closeRightInset: null,
    },
  })
})

it('shows the mobile page heading and focuses the dialog without outlining an action', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={[
          { id: 'running', label: 'Running', count: 1, content: 'Running' },
        ]}
        activeTabId="running"
        onTabChange={vi.fn()}
        onClose={() => undefined}
        sessionTitle="Sample session title"
        size="phone"
      />,
    )
  })

  const backButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Back to the conversation"]',
  )
  const dialog = container.querySelector<HTMLElement>('[role="dialog"]')
  const snapshot = () => ({
    heading: container.querySelector('h1')?.textContent,
    sessionTitle: container.querySelector(
      '.session-status-panel-mobile-heading > span',
    )?.textContent,
    dialog: container
      .querySelector('[role="dialog"]')
      ?.getAttribute('aria-modal'),
    tabLabel: container.querySelector('[role="tab"]')?.textContent,
    backButtonVisible: backButton !== null,
    dialogFocused: document.activeElement === dialog,
    backButtonFocused: document.activeElement === backButton,
    dialogOutline:
      dialog === null ? null : getComputedStyle(dialog).outlineStyle,
    closeButtonVisible:
      container.querySelector('.session-status-panel-close') !== null,
  })
  expect(snapshot()).toEqual({
    heading: 'Session status',
    sessionTitle: 'Sample session title',
    dialog: 'true',
    tabLabel: 'Running1',
    backButtonVisible: true,
    dialogFocused: true,
    backButtonFocused: false,
    dialogOutline: 'none',
    closeButtonVisible: false,
  })
})

it('returns to the conversation when the mobile back action is pressed', () => {
  const onClose = vi.fn()
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={[
          { id: 'running', label: 'Running', count: 1, content: 'Running' },
        ]}
        activeTabId="running"
        onTabChange={vi.fn()}
        onClose={onClose}
        size="phone"
      />,
    )
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '[aria-label="Back to the conversation"]',
      )
      ?.click()
  })
  expect(onClose.mock.calls).toEqual([[]])
})

it('keeps keyboard focus inside the mobile status page', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={[
          { id: 'running', label: 'Running', count: 1, content: 'Running' },
        ]}
        activeTabId="running"
        onTabChange={vi.fn()}
        onClose={() => undefined}
        size="phone"
      />,
    )
  })

  const backButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Back to the conversation"]',
  )
  const tab = container.querySelector<HTMLButtonElement>('[role="tab"]')
  const focusedElements: string[] = []
  const recordFocus = () => {
    const dialog = container.querySelector('[role="dialog"]')
    focusedElements.push(
      document.activeElement === dialog
        ? 'dialog'
        : document.activeElement === backButton
          ? 'back'
          : document.activeElement === tab
            ? 'tab'
            : 'outside',
    )
  }
  recordFocus()
  act(() => {
    document.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Tab',
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  recordFocus()
  act(() => {
    document.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'Tab',
        bubbles: true,
        cancelable: true,
      }),
    )
  })
  recordFocus()

  expect(focusedElements).toEqual(['dialog', 'tab', 'back'])
})
