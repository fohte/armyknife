import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionStatusPanel } from '#components/session-status-panel'

let container: HTMLDivElement
let root: Root | null = null

function renderPanel(activeTabId: string | null, onTabChange = vi.fn()) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionStatusPanel
        tabs={[
          { id: 'running', label: 'Running', count: 0, content: 'Running' },
          { id: 'crit', label: 'crit', count: 1, content: 'Crit content' },
        ]}
        activeTabId={activeTabId}
        onTabChange={onTabChange}
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
