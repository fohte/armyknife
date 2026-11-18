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
}: {
  isMobile: boolean
  pullRequests?: PullRequest[]
}) {
  const [open, setOpen] = useState(false)
  return (
    <SessionPullRequests
      pullRequests={sessionPullRequests}
      repo="example-org/example-repo"
      isMobile={isMobile}
      open={open}
      onOpenChange={setOpen}
    />
  )
}

function render(
  isMobile: boolean,
  sessionPullRequests: PullRequest[] = pullRequests,
) {
  act(() => {
    root?.render(
      <PullRequestHarness
        isMobile={isMobile}
        pullRequests={sessionPullRequests}
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
      container.querySelectorAll<HTMLAnchorElement>('.session-pr-row'),
      (link) => ({
        number: link.querySelector('.session-pr-number')?.textContent ?? null,
        title: (link.querySelector('.session-pr-row-title')?.textContent ?? '')
          .replace(/^#\d+\s*/, '')
          .trim(),
        repo: link.querySelector('.session-pr-row-repo')?.textContent ?? null,
        state:
          link.querySelector(
            isMobile ? '.session-pr-row-mobile-state' : '.session-pr-row-state',
          )?.textContent ?? null,
        ciState:
          link
            .querySelector('.session-pr-ci-dot[role="img"]')
            ?.getAttribute('aria-label') ?? null,
        href: link.getAttribute('href'),
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
          state: 'Open',
          ciState: 'CI running',
          href: 'https://github.com/example-org/example-repo/pull/42',
        },
        {
          number: '#43',
          title: 'Add delivery retry settings',
          repo: 'example-org/example-infra',
          state: 'Open',
          ciState: 'CI passed',
          href: 'https://github.com/example-org/example-infra/pull/43',
        },
        {
          number: '#41',
          title: 'Record delivery attempts',
          repo: 'example-org/example-repo',
          state: '✓ Merged',
          ciState: 'CI status unknown',
          href: 'https://github.com/example-org/example-repo/pull/41',
        },
      ],
    },
    closed: { expanded: 'false', dialogVisible: false },
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
      container.querySelectorAll<HTMLAnchorElement>('.session-pr-row'),
      (link) => ({
        group:
          link.closest('.session-pr-group')?.querySelector('h3')?.textContent ??
          null,
        number: link.querySelector('.session-pr-number')?.textContent ?? null,
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
