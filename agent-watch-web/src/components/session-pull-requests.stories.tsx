import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import type { AsyncActionStatus } from '#async-action-feedback'
import { SessionPullRequests } from '#components/session-pull-requests'
import type { PullRequest } from '#types'

const currentBranchPullRequest: PullRequest = {
  number: 42,
  state: 'Open',
  owner: 'example-org',
  repo: 'example-repo',
  title: 'Show delivery status in the session header',
  url: 'https://github.com/example-org/example-repo/pull/42',
  headSha: 'fixture-head-sha',
  ciState: 'InProgress',
  isCurrentBranch: true,
}

const mergeablePullRequest: PullRequest = {
  number: 44,
  state: 'Open',
  owner: 'example-org',
  repo: 'example-repo',
  title: 'Prepare the release summary',
  url: 'https://github.com/example-org/example-repo/pull/44',
  headSha: 'fixture-mergeable-sha',
  ciState: 'Successful',
  isCurrentBranch: true,
}

const desktopMergeButtonDecorator: Decorator = (Story) => (
  <div
    style={{
      minHeight: 200,
      padding: 16,
      background: 'var(--aw-bg-app)',
      color: 'var(--aw-text-primary)',
    }}
  >
    <Story />
  </div>
)

const otherPullRequests: PullRequest[] = [
  {
    number: 43,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-infra',
    title: 'Add delivery retry settings',
    url: 'https://github.com/example-org/example-infra/pull/43',
    headSha: 'fixture-other-sha',
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
    headSha: 'fixture-merged-sha',
    ciState: 'Unknown',
    isCurrentBranch: false,
  },
]

const meta = {
  component: SessionPullRequests,
  args: {
    pullRequests: [currentBranchPullRequest],
    repo: 'example-org/example-repo',
    isMobile: false,
    open: false,
    onMerge: () => undefined,
    isMerging: (): boolean => false,
    isMergeSuccessful: (): boolean => false,
    onOpenChange: () => undefined,
  },
  decorators: [
    (Story) => (
      <div
        style={{
          minHeight: 120,
          padding: 16,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionPullRequests>

export default meta
type Story = StoryObj<typeof meta>

export const OnePullRequestWithRunningChecks: Story = {
  name: 'The desktop header shows a compact icon for a pull request with running checks',
}

export const MorePullRequestsAreAvailable: Story = {
  name: 'The header counts pull requests opened elsewhere and merged',
  args: {
    pullRequests: [currentBranchPullRequest, ...otherPullRequests],
  },
}

export const AllPullRequestsAreMerged: Story = {
  name: 'The desktop header hides the CI dot when all pull requests are merged.',
  args: {
    pullRequests: [
      { ...currentBranchPullRequest, state: 'Merged' },
      ...otherPullRequests.map((pullRequest) => ({
        ...pullRequest,
        state: 'Merged' as const,
      })),
    ],
  },
}

export const AllPullRequestsAreMergedOnMobile: Story = {
  name: 'The mobile header hides the CI dot when all pull requests are merged.',
  args: {
    pullRequests: [
      { ...currentBranchPullRequest, state: 'Merged' },
      ...otherPullRequests.map((pullRequest) => ({
        ...pullRequest,
        state: 'Merged' as const,
      })),
    ],
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{
          position: 'relative',
          height: 120,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <div className="mobile-conversation-header-actions">
          <Story />
        </div>
      </div>
    ),
  ],
}

export const ChecksAreWaiting: Story = {
  name: 'The header uses a yellow dot while checks are waiting',
  args: {
    pullRequests: [{ ...currentBranchPullRequest, ciState: 'Waiting' }],
  },
}

export const ChecksAreRunning: Story = {
  name: 'The header uses a yellow dot while checks are running',
}

export const ChecksHavePassed: Story = {
  name: 'The header uses a green dot when checks have passed',
  args: {
    pullRequests: [{ ...currentBranchPullRequest, ciState: 'Successful' }],
  },
}

export const ChecksHaveFailed: Story = {
  name: 'The header uses a red dot when checks have failed',
  args: {
    pullRequests: [{ ...currentBranchPullRequest, ciState: 'Failed' }],
  },
}

export const CheckStatusIsUnknown: Story = {
  name: 'The header uses a muted dot when the check status is unknown',
  args: {
    pullRequests: [{ ...currentBranchPullRequest, ciState: 'Unknown' }],
  },
}

export const DesktopPullRequestListIsOpen: Story = {
  name: 'The desktop list shows a reason beside an unmergeable pull request and Merge beside a ready pull request',
  args: {
    pullRequests: [currentBranchPullRequest, ...otherPullRequests],
    open: true,
  },
  decorators: [
    (Story) => (
      <div
        style={{
          minHeight: 460,
          padding: 16,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const DesktopMergeButtonIsOutlined: Story = {
  name: 'The desktop list shows an outlined Merge button for a ready pull request.',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MobileMergeButtonIsOutlined: Story = {
  name: 'The mobile sheet shows a full-height outlined Merge button for a ready pull request.',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
  },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{
          position: 'relative',
          height: 500,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const MergeButtonIsDisabledWhileMerging: Story = {
  name: 'The desktop list keeps the outlined button while a merge is in progress.',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    isMerging: () => true,
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MobilePullRequestSheetIsOpen: Story = {
  name: 'The mobile sheet shows Merge on a ready pull request and Close at the bottom',
  args: {
    pullRequests: [currentBranchPullRequest, ...otherPullRequests],
    isMobile: true,
    open: true,
  },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{
          position: 'relative',
          height: 844,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const AFailingPullRequestCannotBeMerged: Story = {
  name: 'The desktop list explains that a pull request with failing checks cannot be merged',
  args: {
    pullRequests: [{ ...currentBranchPullRequest, ciState: 'Failed' }],
    open: true,
  },
}

export const MobileCompactHeaderButton: Story = {
  name: 'The mobile header shows a compact pull request icon and total count',
  args: {
    pullRequests: [currentBranchPullRequest, ...otherPullRequests],
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div
        className="conversation-pane"
        data-mobile="true"
        style={{
          position: 'relative',
          height: 120,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <div className="mobile-conversation-header-actions">
          <Story />
        </div>
      </div>
    ),
  ],
}

export const MergeSucceededInTheList: Story = {
  name: 'The pull request row reports a successful merge.',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'merged',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const PullRequestWasAlreadyMergedInTheList: Story = {
  name: 'The pull request row reports that it was already merged.',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'alreadyMerged',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MergeIsQueuedInTheList: Story = {
  name: 'The pull request row reports that the merge is queued.',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'queued',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MergeCanBeRetriedInTheList: Story = {
  name: 'The pull request row offers retry after a merge failure.',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'mergeFailed',
    onRetryMerge: () => undefined,
  },
  decorators: [desktopMergeButtonDecorator],
}
