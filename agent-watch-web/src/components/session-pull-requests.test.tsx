import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import {
  AsyncActionFeedbackProvider,
  useAsyncActionStarter,
} from '#async-action-feedback'
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
        onMerge={(pullRequest) => {
          setSessionPullRequests((current) =>
            current.filter(
              (candidate) => candidate.number !== pullRequest.number,
            ),
          )
        }}
        isMerging={() => false}
        isMergeSuccessful={() => false}
        onOpenChange={setOpen}
      />
    </AsyncActionFeedbackProvider>
  )
}

function ToastDismissContents() {
  const [open, setOpen] = useState(false)
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
      <SessionPullRequests
        pullRequests={pullRequests}
        repo="example-org/example-repo"
        isMobile
        open={open}
        onMerge={() => undefined}
        isMerging={() => false}
        isMergeSuccessful={() => false}
        onOpenChange={setOpen}
      />
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

function PullRequestHarness({
  isMobile,
  pullRequests: sessionPullRequests = pullRequests,
  merging = false,
  onMerge = () => undefined,
}: {
  isMobile: boolean
  pullRequests?: PullRequest[]
  merging?: boolean
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
      isMerging={() => merging}
      isMergeSuccessful={() => false}
      onOpenChange={setOpen}
    />
  )
}

function render(
  isMobile: boolean,
  sessionPullRequests: PullRequest[] = pullRequests,
  merging = false,
  onMerge: (pullRequest: PullRequest) => void = () => undefined,
) {
  act(() => {
    root?.render(
      <PullRequestHarness
        isMobile={isMobile}
        pullRequests={sessionPullRequests}
        merging={merging}
        onMerge={onMerge}
      />,
    )
  })
}

function mergeButtonStyle(button: HTMLButtonElement, isMobile: boolean) {
  const style = getComputedStyle(button)
  return {
    height: style.height,
    minHeight: style.minHeight,
    ...(isMobile ? { minWidth: style.minWidth } : {}),
    paddingLeft: style.paddingLeft,
    fontSize: style.fontSize,
    borderTopColor: style.borderTopColor,
    borderTopWidth: style.borderTopWidth,
    backgroundColor: style.backgroundColor,
    color: style.color,
  }
}

function mergingButtonState(button: HTMLButtonElement) {
  const style = getComputedStyle(button)
  return {
    label: button.textContent,
    disabled: button.disabled,
    borderTopColor: style.borderTopColor,
    borderTopWidth: style.borderTopWidth,
    backgroundColor: style.backgroundColor,
    color: style.color,
    opacity: style.opacity,
  }
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
  render(false, pullRequests, false, onMerge)
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

it.each([
  {
    viewport: 'desktop',
    isMobile: false,
    expected: {
      height: '26px',
      minHeight: '26px',
      paddingLeft: '10px',
      fontSize: '12px',
      borderTopColor: 'rgb(63, 63, 70)',
      borderTopWidth: '1px',
      backgroundColor: 'rgba(0, 0, 0, 0)',
      color: 'rgb(250, 250, 250)',
    },
  },
  {
    viewport: 'mobile',
    isMobile: true,
    expected: {
      height: '44px',
      minHeight: '44px',
      minWidth: '72px',
      paddingLeft: '14px',
      fontSize: '13px',
      borderTopColor: 'rgb(63, 63, 70)',
      borderTopWidth: '1px',
      backgroundColor: 'rgba(0, 0, 0, 0)',
      color: 'rgb(250, 250, 250)',
    },
  },
])(
  'uses the designed dimensions for the $viewport Merge button',
  ({ isMobile, expected }) => {
    render(isMobile)
    const trigger = container.querySelector<HTMLButtonElement>(
      '.session-pr-trigger',
    )
    if (trigger === null) throw new Error('Expected pull request trigger')
    act(() => {
      trigger.click()
    })

    const button = container.querySelector<HTMLButtonElement>(
      '.session-pr-row .conversation-pr-merge-button',
    )
    if (button === null) throw new Error('Expected a mergeable pull request')
    expect(mergeButtonStyle(button, isMobile)).toEqual(expected)
  },
)

it('keeps the Merge button outlined and readable while merging', () => {
  render(false, pullRequests, true)
  const trigger = container.querySelector<HTMLButtonElement>(
    '.session-pr-trigger',
  )
  if (trigger === null) throw new Error('Expected pull request trigger')
  act(() => {
    trigger.click()
  })

  const button = container.querySelector<HTMLButtonElement>(
    '.session-pr-row .conversation-pr-merge-button',
  )
  if (button === null) throw new Error('Expected a mergeable pull request')
  expect(mergingButtonState(button)).toEqual({
    label: 'Merging…',
    disabled: true,
    borderTopColor: 'rgb(63, 63, 70)',
    borderTopWidth: '1px',
    backgroundColor: 'rgba(0, 0, 0, 0)',
    color: 'rgb(113, 113, 122)',
    opacity: '1',
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
