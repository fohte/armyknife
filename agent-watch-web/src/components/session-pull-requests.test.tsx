import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionPullRequests } from '#components/session-pull-requests'
import type { PullRequest } from '#types'

const pullRequests: PullRequest[] = [
  {
    number: 42,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-repo',
    title: 'Show delivery status in the session header',
    url: 'https://github.com/example-org/example-repo/pull/42',
    ciState: 'InProgress',
    isCurrentBranch: true,
  },
  {
    number: 43,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-infra',
    title: 'Add delivery retry settings',
    url: 'https://github.com/example-org/example-infra/pull/43',
    ciState: 'Successful',
    headSha: 'fixture-head-sha',
    isCurrentBranch: false,
  },
  {
    number: 41,
    state: 'Merged',
    owner: 'example-org',
    repo: 'example-repo',
    title: 'Record delivery attempts',
    url: 'https://github.com/example-org/example-repo/pull/41',
    ciState: 'Unknown',
    isCurrentBranch: false,
  },
]

let container: HTMLDivElement
let root: Root | null = null

function PullRequestHarness({
  isMobile,
  pullRequests: sessionPullRequests = pullRequests,
  onMerge = () => undefined,
}: {
  isMobile: boolean
  pullRequests?: PullRequest[]
  onMerge?: (pullRequest: PullRequest) => void
}) {
  const [open, setOpen] = useState(false)
  return (
    <SessionPullRequests
      pullRequests={sessionPullRequests}
      repo="example-org/example-repo"
      isMobile={isMobile}
      open={open}
      onMerge={onMerge}
      isMerging={() => false}
      isMergeSuccessful={() => false}
      onOpenChange={setOpen}
    />
  )
}

function render(
  isMobile: boolean,
  sessionPullRequests: PullRequest[] = pullRequests,
  onMerge: (pullRequest: PullRequest) => void = () => undefined,
) {
  act(() => {
    root?.render(
      <PullRequestHarness
        isMobile={isMobile}
        pullRequests={sessionPullRequests}
        onMerge={onMerge}
      />,
    )
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it.each([
  { description: 'desktop', isMobile: false },
  { description: 'mobile', isMobile: true },
])('opens and closes the $description pull request list', ({ isMobile }) => {
  render(isMobile)
  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')

  act(() => {
    trigger.click()
  })

  const openSnapshot = () => ({
    expanded: trigger.getAttribute('aria-expanded'),
    hasPullRequestIcon: trigger.querySelector('.session-pr-icon') !== null,
    count: container.querySelector('.session-pr-count')?.textContent ?? null,
    triggerSize: {
      width: getComputedStyle(trigger).width,
      height: getComputedStyle(trigger).height,
    },
    dialogLabel:
      container.querySelector('[role="dialog"]')?.getAttribute('aria-label') ??
      null,
    mobileModal:
      container.querySelector('[role="dialog"]')?.getAttribute('aria-modal') ??
      null,
    groupNames: Array.from(
      container.querySelectorAll('.session-pr-group h3'),
      (heading) => heading.textContent,
    ),
    pullRequests: Array.from(
      container.querySelectorAll<HTMLElement>('.session-pr-row'),
      (row) => ({
        number: row.querySelector('.session-pr-number')?.textContent ?? null,
        title: (row.querySelector('.session-pr-row-title')?.textContent ?? '')
          .replace(/^#\d+\s*/, '')
          .trim(),
        repo: row.querySelector('.session-pr-row-repo')?.textContent ?? null,
        reason: row.querySelector('.session-pr-row-state')?.textContent ?? null,
        mergeLabel:
          row
            .querySelector('button[aria-label^="Merge PR"]')
            ?.getAttribute('aria-label') ?? null,
        ciState:
          row
            .querySelector('.session-pr-ci-dot[role="img"]')
            ?.getAttribute('aria-label') ?? null,
        href: row.querySelector('a')?.getAttribute('href') ?? null,
      }),
    ),
  })
  const opened = openSnapshot()

  if (isMobile) {
    const closeButton =
      container.querySelector<HTMLButtonElement>('.session-pr-close')
    if (closeButton === null) throw new Error('Expected mobile close button')
    act(() => {
      closeButton.click()
    })
  } else {
    act(() => {
      trigger.click()
    })
  }

  const snapshot = () => ({
    opened,
    closed: {
      expanded: trigger.getAttribute('aria-expanded'),
      dialogVisible: container.querySelector('[role="dialog"]') !== null,
    },
  })
  expect(snapshot()).toEqual({
    opened: {
      expanded: 'true',
      hasPullRequestIcon: true,
      count: '3',
      triggerSize: isMobile
        ? { width: '44px', height: '44px' }
        : { width: '30px', height: '26px' },
      dialogLabel: 'Pull requests from this session',
      mobileModal: isMobile ? 'true' : null,
      groupNames: ['This branch', 'Opened elsewhere', 'Merged'],
      pullRequests: [
        {
          number: '#42',
          title: 'Show delivery status in the session header',
          repo: 'example-org/example-repo',
          reason: 'CI running',
          mergeLabel: null,
          ciState: 'CI running',
          href: 'https://github.com/example-org/example-repo/pull/42',
        },
        {
          number: '#43',
          title: 'Add delivery retry settings',
          repo: 'example-org/example-infra',
          reason: null,
          mergeLabel: 'Merge PR #43',
          ciState: 'CI passed',
          href: 'https://github.com/example-org/example-infra/pull/43',
        },
        {
          number: '#41',
          title: 'Record delivery attempts',
          repo: 'example-org/example-repo',
          reason: '✓ Merged',
          mergeLabel: null,
          ciState: 'CI status unknown',
          href: 'https://github.com/example-org/example-repo/pull/41',
        },
      ],
    },
    closed: { expanded: 'false', dialogVisible: false },
  })
})

it.each([
  { description: 'desktop outside pointer press', isMobile: false, key: false },
  { description: 'desktop Escape', isMobile: false, key: true },
  { description: 'mobile backdrop press', isMobile: true, key: false },
  { description: 'mobile Escape', isMobile: true, key: true },
])('closes the pull request list on $description', ({ isMobile, key }) => {
  render(isMobile)
  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')
  act(() => {
    trigger.click()
  })

  act(() => {
    if (key) {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    } else if (isMobile) {
      const backdrop = container.querySelector('.session-pr-backdrop')
      if (backdrop === null) throw new Error('Expected mobile PR backdrop')
      backdrop.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    } else {
      document.body.dispatchEvent(
        new PointerEvent('pointerdown', { bubbles: true }),
      )
    }
  })

  const snapshot = () => ({
    expanded: trigger.getAttribute('aria-expanded'),
    popoverVisible: container.querySelector('.session-pr-popover') !== null,
  })
  expect(snapshot()).toEqual({ expanded: 'false', popoverVisible: false })
})

it('keeps the pull request list open when merging a row', () => {
  const onMerge = vi.fn()
  render(false, pullRequests, onMerge)
  act(() => {
    container.querySelector<HTMLButtonElement>('.session-pr-trigger')?.click()
  })
  const mergeButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Merge PR #43"]',
  )
  if (mergeButton === null) throw new Error('Expected merge action')
  act(() => {
    mergeButton.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true }),
    )
    mergeButton.click()
  })

  const snapshot = () => ({
    calls: onMerge.mock.calls,
    expanded: container
      .querySelector('.session-pr-trigger')
      ?.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    calls: [[pullRequests[1]]],
    expanded: 'true',
  })
})

it('lists a merged current-branch pull request only in Merged', () => {
  render(false, [
    {
      number: 42,
      state: 'Merged',
      owner: 'example-org',
      repo: 'example-repository',
      title: 'Complete the session header change',
      url: 'https://github.com/example-org/example-repository/pull/42',
      ciState: 'Successful',
      isCurrentBranch: true,
    },
    {
      number: 43,
      state: 'Open',
      owner: 'example-org',
      repo: 'example-infrastructure',
      title: 'Add session header diagnostics',
      url: 'https://github.com/example-org/example-infrastructure/pull/43',
      ciState: 'InProgress',
      isCurrentBranch: false,
    },
  ])

  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')
  act(() => {
    trigger.click()
  })

  const list = () => ({
    groupNames: Array.from(
      container.querySelectorAll('.session-pr-group h3'),
      (heading) => heading.textContent,
    ),
    pullRequests: Array.from(
      container.querySelectorAll<HTMLElement>('.session-pr-row'),
      (row) => ({
        group:
          row.closest('.session-pr-group')?.querySelector('h3')?.textContent ??
          null,
        number: row.querySelector('.session-pr-number')?.textContent ?? null,
      }),
    ),
  })
  expect(list()).toEqual({
    groupNames: ['Opened elsewhere', 'Merged'],
    pullRequests: [
      { group: 'Opened elsewhere', number: '#43' },
      { group: 'Merged', number: '#42' },
    ],
  })
})

it('shows the CI failure reason without a merge action', () => {
  render(false, [
    {
      number: 44,
      state: 'Open',
      owner: 'example-org',
      repo: 'example-repository',
      title: 'Update the release checks',
      url: 'https://github.com/example-org/example-repository/pull/44',
      headSha: 'fixture-head-sha',
      ciState: 'Failed',
      isCurrentBranch: true,
    },
  ])

  act(() => {
    container.querySelector<HTMLButtonElement>('.session-pr-trigger')?.click()
  })

  const row = container.querySelector<HTMLElement>('.session-pr-row')
  expect(
    row === null
      ? null
      : {
          reason: row.querySelector('.session-pr-row-state')?.textContent,
          mergeButton:
            row.querySelector('button[aria-label^="Merge PR"]') !== null,
        },
  ).toEqual({ reason: 'CI failed', mergeButton: false })
})

it('shows the missing head SHA reason without a merge action', () => {
  render(false, [
    {
      number: 45,
      state: 'Open',
      owner: 'example-org',
      repo: 'example-repository',
      title: 'Verify the release checks',
      url: 'https://github.com/example-org/example-repository/pull/45',
      headSha: null,
      ciState: 'Successful',
      isCurrentBranch: true,
    },
  ])

  act(() => {
    container.querySelector<HTMLButtonElement>('.session-pr-trigger')?.click()
  })

  const row = container.querySelector<HTMLElement>('.session-pr-row')
  expect(
    row === null
      ? null
      : {
          reason: row.querySelector('.session-pr-row-state')?.textContent,
          mergeButton:
            row.querySelector('button[aria-label^="Merge PR"]') !== null,
        },
  ).toEqual({ reason: 'Head SHA unavailable', mergeButton: false })
})

it('dims the pull request icon when every pull request is merged', () => {
  render(false, [
    {
      number: 42,
      state: 'Merged',
      owner: 'example-org',
      repo: 'example-project',
      title: 'Complete the delivery update',
      url: 'https://github.com/example-org/example-project/pull/42',
      ciState: 'Successful',
      isCurrentBranch: true,
    },
  ])

  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')
  const icon = trigger.querySelector<SVGElement>('.session-pr-icon')
  if (icon === null) throw new Error('Expected pull request icon')
  const count = container.querySelector<HTMLElement>('.session-pr-count')
  if (count === null) throw new Error('Expected pull request count')

  const snapshot = () => ({
    allMerged: trigger.classList.contains('is-all-merged'),
    iconMatchesTriggerColor:
      getComputedStyle(icon).color === getComputedStyle(trigger).color,
    countMatchesTriggerColor:
      getComputedStyle(count).color === getComputedStyle(trigger).color,
  })
  expect(snapshot()).toEqual({
    allMerged: true,
    iconMatchesTriggerColor: true,
    countMatchesTriggerColor: true,
  })
})
