import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { InboxView } from '#components/inbox-view'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.style.width = '320px'
  document.body.append(container)
  root = createRoot(container)
  act(() => {
    root?.render(
      <InboxView
        sort="priority"
        sortStorageAvailable
        onSortChange={() => undefined}
        sessions={[]}
        selectedId={null}
        loaded
        error={null}
        connectionError={null}
        lastUpdatedAt={0}
        showAllInProgress={false}
        onShowMore={() => undefined}
        showAllWaiting={false}
        onShowMoreWaiting={() => undefined}
        onSelect={() => undefined}
        laterExpanded={false}
        onToggleLater={() => undefined}
        onToggleToday={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        isMobile={false}
        inProgressExpanded={false}
        onToggleInProgress={() => undefined}
        waitingExpanded
        onToggleWaiting={() => undefined}
        doneExpanded={false}
        onToggleDone={() => undefined}
      />,
    )
  })
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('wraps the keyboard shortcuts in a narrow Inbox footer', () => {
  const footer = container.querySelector<HTMLElement>('.inbox-footer')
  if (footer === null) throw new Error('Inbox shortcut footer was not rendered')

  const snapshot = () => ({
    overflowsHorizontally: footer.scrollWidth > footer.clientWidth,
    wraps:
      new Set(
        Array.from(
          footer.children,
          (child) => child.getBoundingClientRect().top,
        ),
      ).size > 1,
    growsToFit: footer.clientHeight > 32,
  })

  expect(snapshot()).toEqual({
    overflowsHorizontally: false,
    wraps: true,
    growsToFit: true,
  })
})
