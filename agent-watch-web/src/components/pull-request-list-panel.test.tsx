import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { PullRequestListPanel } from '#components/pull-request-list-panel'
import type { SessionPullRequest } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const pullRequests: SessionPullRequest[] = [
  {
    number: 43,
    owner: null,
    repo: null,
    title: 'Keep active work visible',
    url: 'https://github.com/sample-org/sample-project/pull/43',
    headSha: null,
    state: 'Open',
    ciState: 'Successful',
    reviewStatus: 'done',
  },
  {
    number: 42,
    owner: null,
    repo: null,
    title: 'Finish an earlier change',
    url: 'https://github.com/sample-org/sample-project/pull/42',
    headSha: null,
    state: 'Merged',
    ciState: 'Unknown',
    reviewStatus: 'done',
  },
  {
    number: 41,
    owner: null,
    repo: null,
    title: 'Close an abandoned change',
    url: 'https://github.com/sample-org/sample-project/pull/41',
    headSha: null,
    state: 'Closed',
    ciState: 'Unknown',
    reviewStatus: 'not_started',
  },
]

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
})

it('collapses merged and closed pull requests until requested', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <PullRequestListPanel
        sessionId="sample-session"
        pullRequests={pullRequests}
        loading={false}
        error={null}
        onRefresh={() => undefined}
      />,
    )
  })

  const visibleTitles = () =>
    Array.from(
      container.querySelectorAll('.pull-request-list-title'),
      (title) => title.textContent,
    )
  const toggle = container.querySelector<HTMLButtonElement>(
    '.pull-request-list-completed-toggle',
  )
  const collapsedSnapshot = {
    titles: visibleTitles(),
    toggle: toggle?.textContent,
    expanded: toggle?.getAttribute('aria-expanded'),
  }

  act(() => toggle?.click())

  const expandedSnapshot = {
    titles: visibleTitles(),
    toggle: toggle?.textContent,
    expanded: toggle?.getAttribute('aria-expanded'),
  }

  const snapshot = () => [collapsedSnapshot, expandedSnapshot]
  expect(snapshot()).toEqual([
    {
      titles: ['#43Keep active work visible'],
      toggle: 'Show 2 completed',
      expanded: 'false',
    },
    {
      titles: [
        '#43Keep active work visible',
        '#42Finish an earlier change',
        '#41Close an abandoned change',
      ],
      toggle: 'Hide 2 completed',
      expanded: 'true',
    },
  ])
})

it('keeps the list visible without a refresh indicator while loading', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <PullRequestListPanel
        sessionId="sample-session"
        pullRequests={pullRequests}
        loading={true}
        error={null}
        onRefresh={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    titles: Array.from(
      container.querySelectorAll('.pull-request-list-title'),
      (title) => title.textContent,
    ),
    refreshing: container.querySelector('.pull-request-list-refreshing'),
  })
  expect(snapshot()).toEqual({
    titles: ['#43Keep active work visible'],
    refreshing: null,
  })
})
