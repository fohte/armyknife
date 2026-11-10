import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { AgentWatchNav } from '#components/agent-watch-nav'

let container: HTMLDivElement
let root: Root | null = null

function renderNav(
  workTypeNavGroups: ComponentProps<typeof AgentWatchNav>['workTypeNavGroups'],
  onSelectSession: (id: string) => void = () => undefined,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <AgentWatchNav
        activeView="inbox"
        needsActionCount={0}
        workTypeNavGroups={workTypeNavGroups}
        onViewChange={() => undefined}
        todaySessions={[]}
        onSelectToday={() => undefined}
        onSelectSession={onSelectSession}
        onOpenPalette={() => undefined}
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
