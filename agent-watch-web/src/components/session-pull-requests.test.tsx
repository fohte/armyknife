import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  AsyncActionFeedbackProvider,
  type AsyncActionStatus,
  useAsyncActionStarter,
} from '#async-action-feedback'
import { LaterMenu } from '#components/later-menu'
import { SessionPullRequests } from '#components/session-pull-requests'
import type { PullRequest } from '#types'
import { useInboxFilterShortcut } from '#use-inbox-filter-shortcut'

const pullRequests: PullRequest[] = [
  {
    number: 42,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-repo',
    title: 'Show delivery status',
    url: 'https://github.com/example-org/example-repo/pull/42',
    ciState: 'InProgress',
    isCurrentBranch: true,
  },
  {
    number: 43,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-infra',
    title: 'Add retry settings',
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
    title: 'Record attempts',
    url: 'https://github.com/example-org/example-repo/pull/41',
    ciState: 'Unknown',
    isCurrentBranch: false,
  },
]

const openPullRequestElsewhere: PullRequest = {
  number: 817,
  state: 'Open',
  owner: 'fixture-team',
  repo: 'another-session-demo',
  title: 'Keep checks visible for an open pull request',
  url: 'https://github.com/fixture-team/another-session-demo/pull/817',
  ciState: 'InProgress',
  isCurrentBranch: false,
}

let container: HTMLDivElement
let root: Root | null = null
const innerHeightDescriptor = Object.getOwnPropertyDescriptor(
  window,
  'innerHeight',
)

function ToastOffsetHarness() {
  const [open, setOpen] = useState(false)
  const [sessionPullRequests, setSessionPullRequests] = useState(pullRequests)
  return (
    <AsyncActionFeedbackProvider>
      <button
        className="remove-pull-requests"
        onClick={() => {
          setSessionPullRequests([])
        }}
        type="button"
      >
        Remove pull requests
      </button>
      <SessionPullRequests
        pullRequests={sessionPullRequests}
        repo="example-org/example-repo"
        isMobile
        open={open}
        onOpenChange={setOpen}
      />
    </AsyncActionFeedbackProvider>
  )
}

function ToastDismissContents() {
  const [open, setOpen] = useState(false)
  const [laterMenuOpen, setLaterMenuOpen] = useState(false)
  const startAsyncAction = useAsyncActionStarter(() => undefined)
  return (
    <>
      <button
        className="start-failed-merge"
        onClick={() => {
          startAsyncAction({
            kind: 'merge',
            sessionId: 'fixture-session',
            sessionTitle: 'Fixture session',
            pullRequestNumber: 42,
            run: () =>
              Promise.resolve({
                status: 'mergeFailed',
                error: 'The pull request head changed.',
              }),
          })
        }}
        type="button"
      >
        Start failed merge
      </button>
      <button
        className="open-later-menu"
        onClick={() => {
          setLaterMenuOpen(true)
        }}
        type="button"
      >
        Open Later menu
      </button>
      <SessionPullRequests
        pullRequests={pullRequests}
        repo="example-org/example-repo"
        isMobile
        open={open}
        onOpenChange={setOpen}
      />
      {laterMenuOpen ? (
        <LaterMenu
          sessionTitle="Fixture session"
          isLater={false}
          laterTimes={null}
          laterTimesError
          isSubmitting={false}
          onSelect={() => undefined}
          onBackNow={() => undefined}
          onClose={() => {
            setLaterMenuOpen(false)
          }}
        />
      ) : null}
    </>
  )
}

function ToastDismissHarness() {
  return (
    <AsyncActionFeedbackProvider>
      <ToastDismissContents />
    </AsyncActionFeedbackProvider>
  )
}

function LaterFilterShortcutHarness({
  onOpenFilter,
}: {
  onOpenFilter: () => void
}) {
  const [filterOpen, setFilterOpen] = useState(false)
  const openFilter = () => {
    onOpenFilter()
    setFilterOpen(true)
  }
  useInboxFilterShortcut(filterOpen, openFilter)

  return (
    <>
      {filterOpen ? (
        <div aria-label="Filter inbox" aria-modal="true" role="dialog" />
      ) : null}
      <LaterMenu
        isMobile
        sessionTitle="Fixture session"
        isLater={false}
        laterTimes={null}
        laterTimesError
        isSubmitting={false}
        onSelect={() => undefined}
        onBackNow={() => undefined}
        onClose={() => undefined}
      />
    </>
  )
}

function PullRequestHarness({
  isMobile,
  pullRequests: sessionPullRequests = pullRequests,
  mergeStatus = () => null,
}: {
  isMobile: boolean
  pullRequests?: PullRequest[]
  mergeStatus?: (pullRequest: PullRequest) => AsyncActionStatus | null
}) {
  const [open, setOpen] = useState(false)
  return (
    <SessionPullRequests
      pullRequests={sessionPullRequests}
      repo="example-org/example-repo"
      isMobile={isMobile}
      open={open}
      mergeStatus={mergeStatus}
      onOpenChange={setOpen}
    />
  )
}

function render(
  isMobile: boolean,
  sessionPullRequests: PullRequest[] = pullRequests,
  mergeStatus: (pullRequest: PullRequest) => AsyncActionStatus | null = () =>
    null,
) {
  act(() => {
    root?.render(
      <PullRequestHarness
        isMobile={isMobile}
        pullRequests={sessionPullRequests}
        mergeStatus={mergeStatus}
      />,
    )
  })
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.style.setProperty('--aw-border-control', '#3f3f46')
  container.style.setProperty('--aw-bg-pressed', '#262626')
  container.style.setProperty('--aw-text-primary', '#fafafa')
  container.style.setProperty('--aw-text-disabled', '#71717a')
  container.style.setProperty('--aw-radius', '0')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  if (innerHeightDescriptor === undefined) {
    Reflect.deleteProperty(window, 'innerHeight')
  } else {
    Object.defineProperty(window, 'innerHeight', innerHeightDescriptor)
  }
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it.each([
  { description: 'desktop', isMobile: false },
  { description: 'mobile', isMobile: true },
])('opens and closes the $description pull request list', ({ isMobile }) => {
  const closedPullRequest: PullRequest = {
    number: 44,
    state: 'Closed',
    owner: 'example-org',
    repo: 'example-infra',
    title: 'Close an old pull request',
    url: 'https://github.com/example-org/example-infra/pull/44',
    ciState: 'Unknown',
    isCurrentBranch: false,
  }
  render(isMobile, [...pullRequests, closedPullRequest])
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
        mobileState:
          row.querySelector('.session-pr-row-mobile-state')?.textContent ??
          null,
        mobileStateAriaHidden:
          row
            .querySelector('.session-pr-row-mobile-state')
            ?.getAttribute('aria-hidden') ?? null,
        reason: row.querySelector('.session-pr-row-state')?.textContent ?? null,
        hasAction: row.querySelector('button[aria-label^="Merge PR"]') !== null,
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
      count: '4',
      triggerSize: isMobile
        ? { width: '44px', height: '44px' }
        : { width: '30px', height: '26px' },
      dialogLabel: 'Pull requests from this session',
      mobileModal: isMobile ? 'true' : null,
      groupNames: ['This branch', 'Opened elsewhere', 'Merged'],
      pullRequests: [
        {
          number: '#42',
          title: 'Show delivery status',
          repo: 'example-org/example-repo',
          mobileState: isMobile ? '· Open' : null,
          mobileStateAriaHidden: null,
          reason: 'CI running',
          hasAction: false,
          ciState: 'CI running',
          href: 'https://github.com/example-org/example-repo/pull/42',
        },
        {
          number: '#43',
          title: 'Add retry settings',
          repo: 'example-org/example-infra',
          mobileState: isMobile ? '· Open' : null,
          mobileStateAriaHidden: null,
          reason: 'Merge',
          hasAction: true,
          ciState: 'CI passed',
          href: 'https://github.com/example-org/example-infra/pull/43',
        },
        {
          number: '#44',
          title: 'Close an old pull request',
          repo: 'example-org/example-infra',
          mobileState: isMobile ? '· Closed' : null,
          mobileStateAriaHidden: null,
          reason: isMobile ? null : 'Closed',
          hasAction: false,
          ciState: 'CI status unknown',
          href: 'https://github.com/example-org/example-infra/pull/44',
        },
        {
          number: '#41',
          title: 'Record attempts',
          repo: 'example-org/example-repo',
          mobileState: isMobile ? '· Merged' : null,
          mobileStateAriaHidden: null,
          reason: '✓ Merged',
          hasAction: false,
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

it('shows a disabled merge action while a pull request is merging', () => {
  render(false, pullRequests, (pullRequest) =>
    pullRequest.number === 43 ? 'merging' : null,
  )
  act(() => {
    container.querySelector<HTMLButtonElement>('.session-pr-trigger')?.click()
  })
  const mergeableRow = Array.from(
    container.querySelectorAll<HTMLElement>('.session-pr-row'),
  ).find(
    (row) => row.querySelector('.session-pr-number')?.textContent === '#43',
  )

  const snapshot = () => ({
    rowStatus: mergeableRow?.querySelector('.session-pr-row-state')
      ?.textContent,
    hasMergeAction: mergeableRow?.querySelector('button') !== null,
    actionDisabled:
      mergeableRow?.querySelector<HTMLButtonElement>('button')?.disabled ??
      false,
    expanded: container
      .querySelector('.session-pr-trigger')
      ?.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    rowStatus: 'Merging…',
    hasMergeAction: true,
    actionDisabled: true,
    expanded: 'true',
  })
})

it.each([
  { description: 'desktop', isMobile: false },
  { description: 'phone', isMobile: true },
])(
  'shows a failure label and retry action in the $description list row',
  ({ isMobile }) => {
    const mergeablePullRequest = pullRequests.find(
      (pullRequest) => pullRequest.number === 43,
    )
    if (mergeablePullRequest === undefined) {
      throw new Error('Expected a mergeable pull request')
    }
    render(isMobile, [mergeablePullRequest], () => 'mergeFailed')

    const trigger = container.querySelector<HTMLButtonElement>(
      '.session-pr-trigger',
    )
    if (trigger === null) throw new Error('Expected pull request trigger')
    act(() => {
      trigger.click()
    })
    const row = container.querySelector<HTMLElement>('.session-pr-row')

    const snapshot = () => {
      const label = row?.querySelector<HTMLElement>(
        '.pr-merge-entry-failure-label',
      )
      const action = row?.querySelector<HTMLButtonElement>('button')
      return {
        label: label?.textContent.trim() ?? null,
        labelRole: label?.getAttribute('role') ?? null,
        icon: label?.querySelector('svg')?.getAttribute('aria-hidden') ?? null,
        action: action?.textContent ?? null,
        actionLabel: action?.getAttribute('aria-label') ?? null,
        actionDisabled: action?.disabled ?? false,
      }
    }
    expect(snapshot()).toEqual({
      label: 'Failed',
      labelRole: 'status',
      icon: 'true',
      action: 'Retry',
      actionLabel: 'Retry merging PR #43',
      actionDisabled: false,
    })
  },
)

it.each([
  {
    description: 'a merge on desktop',
    isMobile: false,
    status: 'merged' as const,
    expectedStatus: '✓ Merged',
  },
  {
    description: 'a merge on the phone',
    isMobile: true,
    status: 'merged' as const,
    expectedStatus: '✓ Merged',
  },
  {
    description: 'an already-completed merge on desktop',
    isMobile: false,
    status: 'alreadyMerged' as const,
    expectedStatus: '✓ Already merged',
  },
  {
    description: 'an already-completed merge on the phone',
    isMobile: true,
    status: 'alreadyMerged' as const,
    expectedStatus: '✓ Already merged',
  },
])(
  'shows the merged row status and marker for $description',
  ({ isMobile, status, expectedStatus }) => {
    const mergeablePullRequest = pullRequests.find(
      (pullRequest) => pullRequest.number === 43,
    )
    if (mergeablePullRequest === undefined) {
      throw new Error('Expected a mergeable pull request')
    }
    render(isMobile, [mergeablePullRequest], () => status)

    const trigger = container.querySelector<HTMLButtonElement>(
      '.session-pr-trigger',
    )
    if (trigger === null) throw new Error('Expected pull request trigger')
    act(() => {
      trigger.click()
    })
    const row = container.querySelector<HTMLElement>('.session-pr-row')

    const snapshot = () => ({
      status: row?.querySelector('.session-pr-row-state')?.textContent ?? null,
      mobileState:
        row?.querySelector('.session-pr-row-mobile-state')?.textContent ?? null,
      ciDotClass: row?.querySelector('.session-pr-ci-dot')?.className ?? null,
      actionTag: row?.querySelector('.pr-merge-entry')?.tagName ?? null,
      hasButton: row?.querySelector('button') !== null,
    })
    expect(snapshot()).toEqual({
      status: expectedStatus,
      mobileState: isMobile
        ? status === 'merged'
          ? '· Merged'
          : '· Already merged'
        : null,
      ciDotClass: 'session-pr-ci-dot is-passed is-merged',
      actionTag: 'SPAN',
      hasButton: false,
    })
  },
)

it.each([
  {
    description:
      'a merge in progress on another pull request in the desktop header',
    isMobile: false,
    status: 'merging' as const,
    expectedWidth: '46px',
    expectedMark: 'session-pr-merge-mark is-inline is-merging',
    expectedPosition: 'static',
    expectedFailure: false,
  },
  {
    description: 'a failed merge on another pull request in the desktop header',
    isMobile: false,
    status: 'mergeFailed' as const,
    expectedWidth: '48px',
    expectedMark: 'session-pr-merge-mark is-inline is-failed',
    expectedPosition: 'static',
    expectedFailure: true,
  },
  {
    description:
      'a merge in progress on another pull request in the phone header',
    isMobile: true,
    status: 'merging' as const,
    expectedWidth: '44px',
    expectedMark: 'session-pr-merge-mark is-mobile is-merging',
    expectedPosition: 'absolute',
    expectedFailure: false,
  },
  {
    description: 'a failed merge on another pull request in the phone header',
    isMobile: true,
    status: 'mergeFailed' as const,
    expectedWidth: '44px',
    expectedMark: 'session-pr-merge-mark is-mobile is-failed',
    expectedPosition: 'absolute',
    expectedFailure: true,
  },
])(
  'marks the PR button for $description',
  ({
    isMobile,
    status,
    expectedMark,
    expectedFailure,
    expectedWidth,
    expectedPosition,
  }) => {
    render(isMobile, pullRequests, (pullRequest) =>
      pullRequest.number === 43 ? status : null,
    )

    const trigger = container.querySelector<HTMLButtonElement>(
      '.session-pr-trigger',
    )
    if (trigger === null) throw new Error('Expected pull request trigger')

    const snapshot = () => ({
      triggerWidth: getComputedStyle(trigger).width,
      hasInlineMergeMark: trigger.classList.contains('is-merge-mark-inline'),
      isMobile: trigger.classList.contains('session-pr-trigger--mobile'),
      hasFailureBorder: trigger.classList.contains('is-merge-failed'),
      mergeMark:
        trigger
          .querySelector('.session-pr-merge-mark')
          ?.getAttribute('class') ?? null,
      mergeMarkPosition: getComputedStyle(
        trigger.querySelector('.session-pr-merge-mark') ?? trigger,
      ).position,
    })
    expect(snapshot()).toEqual({
      triggerWidth: expectedWidth,
      hasInlineMergeMark: !isMobile,
      isMobile,
      hasFailureBorder: expectedFailure,
      mergeMark: expectedMark,
      mergeMarkPosition: expectedPosition,
    })
  },
)

it('hides the CI dot after the representative pull request merges', () => {
  render(false, pullRequests, (pullRequest) =>
    pullRequest.number === 42 ? 'merged' : null,
  )

  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')

  const snapshot = () => ({
    mergeMark: trigger.querySelector('.session-pr-merge-mark') !== null,
    ciDot: trigger.querySelector('.session-pr-trigger-ci-dot') !== null,
  })
  expect(snapshot()).toEqual({ mergeMark: false, ciDot: false })
})

it('ignores stale merge feedback for non-open pull requests', () => {
  const currentPullRequest = pullRequests.find(({ number }) => number === 42)
  if (currentPullRequest === undefined) {
    throw new Error('Expected current pull request')
  }
  const openElsewherePullRequest = pullRequests.find(
    ({ number }) => number === 43,
  )
  if (openElsewherePullRequest === undefined) {
    throw new Error('Expected pull request opened elsewhere')
  }
  const mergedPullRequest: PullRequest = {
    ...currentPullRequest,
    state: 'Merged',
  }
  const closedPullRequest: PullRequest = {
    ...openElsewherePullRequest,
    number: 44,
    state: 'Closed',
    isCurrentBranch: false,
  }
  render(
    false,
    [mergedPullRequest, ...pullRequests.slice(1), closedPullRequest],
    (pullRequest) => {
      if (pullRequest.number === mergedPullRequest.number) return 'mergeFailed'
      if (pullRequest.number === closedPullRequest.number) return 'merging'
      return null
    },
  )

  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')
  act(() => {
    trigger.click()
  })

  const snapshot = () => ({
    hasFailureBorder: trigger.classList.contains('is-merge-failed'),
    mergeMark: trigger.querySelector('.session-pr-merge-mark') !== null,
    ciDot: trigger.querySelector('.session-pr-trigger-ci-dot') !== null,
    rows: Array.from(
      container.querySelectorAll<HTMLElement>('.session-pr-row'),
      (row) => row.querySelector('.session-pr-row-state')?.textContent ?? null,
    ),
  })
  expect(snapshot()).toEqual({
    hasFailureBorder: false,
    mergeMark: false,
    ciDot: false,
    rows: ['Merge', 'Closed', '✓ Merged', '✓ Merged'],
  })
})

it('updates the phone toast offset with sheet geometry and clears stale offsets', () => {
  let sheetTop = 500
  Object.defineProperty(window, 'innerHeight', {
    configurable: true,
    value: 844,
  })
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(
    function (this: HTMLElement) {
      return this.classList.contains('session-pr-list')
        ? new DOMRect(0, sheetTop, 320, 300)
        : new DOMRect()
    },
  )

  act(() => {
    root?.render(<ToastOffsetHarness />)
  })
  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  const stack = container.querySelector<HTMLElement>(
    '.async-action-toast-stack',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')
  if (stack === null) throw new Error('Expected toast host')

  const offsets: Array<{ bottom: string; maxHeight: string }> = []
  const recordOffset = () => {
    offsets.push({
      bottom: stack.style.bottom,
      maxHeight: stack.style.maxHeight,
    })
  }

  act(() => {
    trigger.click()
  })
  recordOffset()

  sheetTop = 460
  Object.defineProperty(window, 'innerHeight', {
    configurable: true,
    value: 800,
  })
  act(() => {
    window.dispatchEvent(new Event('resize'))
  })
  recordOffset()

  act(() => {
    container.querySelector<HTMLButtonElement>('.session-pr-close')?.click()
  })
  recordOffset()

  act(() => {
    trigger.click()
  })
  recordOffset()
  act(() => {
    container.querySelector<HTMLButtonElement>('.remove-pull-requests')?.click()
  })
  recordOffset()

  expect(offsets).toEqual([
    {
      bottom: '352px',
      maxHeight: 'calc(100dvh - 368px)',
    },
    {
      bottom: '348px',
      maxHeight: 'calc(100dvh - 364px)',
    },
    { bottom: '', maxHeight: '' },
    {
      bottom: '348px',
      maxHeight: 'calc(100dvh - 364px)',
    },
    { bottom: '', maxHeight: '' },
  ])
})

it('keeps the phone pull request sheet open when scrolling a toast', async () => {
  act(() => {
    root?.render(<ToastDismissHarness />)
  })

  await act(async () => {
    container.querySelector<HTMLButtonElement>('.start-failed-merge')?.click()
    await Promise.resolve()
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.session-pr-trigger')?.click()
  })

  const toast = container.querySelector<HTMLElement>(
    '.async-action-toast-stack .async-action-item',
  )
  if (toast === null) throw new Error('Expected a failed merge toast')
  act(() => {
    toast.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })

  const snapshot = () => ({
    expanded: container
      .querySelector('.session-pr-trigger')
      ?.getAttribute('aria-expanded'),
    dialogVisible: container.querySelector('.session-pr-list--mobile') !== null,
  })
  expect(snapshot()).toEqual({ expanded: 'true', dialogVisible: true })
})

it('closes an unrelated menu when a toast receives a pointerdown', async () => {
  act(() => {
    root?.render(<ToastDismissHarness />)
  })
  await act(async () => {
    container.querySelector<HTMLButtonElement>('.start-failed-merge')?.click()
    await Promise.resolve()
  })
  act(() => {
    container.querySelector<HTMLButtonElement>('.open-later-menu')?.click()
  })

  const toast = container.querySelector<HTMLElement>(
    '.async-action-toast-stack .async-action-item',
  )
  if (toast === null) throw new Error('Expected a failed merge toast')
  act(() => {
    toast.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
  })

  const snapshot = () => ({
    laterMenuVisible: container.querySelector('.later-menu--mobile') !== null,
    toastVisible:
      container.querySelector(
        '.async-action-toast-stack .async-action-item',
      ) !== null,
  })
  expect(snapshot()).toEqual({ laterMenuVisible: false, toastVisible: true })
})

it('does not open the Inbox filter shortcut while Later is active', () => {
  const onOpenFilter = vi.fn()
  act(() => {
    root?.render(<LaterFilterShortcutHarness onOpenFilter={onOpenFilter} />)
  })
  act(() => {
    window.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: '/',
        bubbles: true,
        cancelable: true,
      }),
    )
  })

  const snapshot = () => ({
    filterOpenCalls: onOpenFilter.mock.calls,
    dialogs: Array.from(
      container.querySelectorAll<HTMLElement>('[role="dialog"]'),
      (dialog) => dialog.getAttribute('aria-label'),
    ),
  })
  expect(snapshot()).toEqual({
    filterOpenCalls: [],
    dialogs: ['Later'],
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

it('shows the CI failure reason as read-only row status text', () => {
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
          hasMergeAction:
            row.querySelector('button[aria-label^="Merge PR"]') !== null,
        },
  ).toEqual({ reason: 'CI failing', hasMergeAction: false })
})

it('shows the missing head SHA reason as read-only row status text', () => {
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
          hasMergeAction:
            row.querySelector('button[aria-label^="Merge PR"]') !== null,
        },
  ).toEqual({ reason: 'Head SHA unavailable', hasMergeAction: false })
})

it.each([
  {
    description: 'desktop with an open pull request',
    isMobile: false,
    state: 'Open' as const,
    expectedDotClass: 'session-pr-trigger-ci-dot is-passed',
    expectedAllMerged: false,
    includeOpenElsewhere: false,
  },
  {
    description: 'mobile with an open pull request',
    isMobile: true,
    state: 'Open' as const,
    expectedDotClass: 'session-pr-trigger-ci-dot is-passed',
    expectedAllMerged: false,
    includeOpenElsewhere: false,
  },
  {
    description: 'desktop when all pull requests are merged',
    isMobile: false,
    state: 'Merged' as const,
    expectedDotClass: null,
    expectedAllMerged: true,
    includeOpenElsewhere: false,
  },
  {
    description: 'mobile when all pull requests are merged',
    isMobile: true,
    state: 'Merged' as const,
    expectedDotClass: null,
    expectedAllMerged: true,
    includeOpenElsewhere: false,
  },
  {
    description: 'desktop with a merged representative and another open PR',
    isMobile: false,
    state: 'Merged' as const,
    expectedDotClass: null,
    expectedAllMerged: false,
    includeOpenElsewhere: true,
  },
])(
  'shows the expected CI dot for the $description',
  ({
    isMobile,
    state,
    expectedDotClass,
    expectedAllMerged,
    includeOpenElsewhere,
  }) => {
    render(isMobile, [
      {
        number: 1721,
        state,
        owner: 'fixture-team',
        repo: 'session-header-demo',
        title: 'Demonstrate the session pull request header',
        url: 'https://github.com/fixture-team/session-header-demo/pull/1721',
        ciState: 'Successful',
        isCurrentBranch: true,
      },
      ...(includeOpenElsewhere ? [openPullRequestElsewhere] : []),
    ])

    const trigger = container.querySelector<HTMLButtonElement>(
      '.session-pr-trigger',
    )
    if (trigger === null) throw new Error('Expected pull request trigger')

    const snapshot = () => ({
      dotClass:
        trigger
          .querySelector('.session-pr-trigger-ci-dot')
          ?.getAttribute('class') ?? null,
      allMerged: trigger.classList.contains('is-all-merged'),
    })
    expect(snapshot()).toEqual({
      dotClass: expectedDotClass,
      allMerged: expectedAllMerged,
    })
  },
)
