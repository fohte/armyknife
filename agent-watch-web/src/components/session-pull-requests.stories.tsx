import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import type { AsyncActionStatus } from '#async-action-feedback'
import { SessionPullRequests } from '#components/session-pull-requests'
import type { PullRequest } from '#types'

const currentBranchPullRequest: PullRequest = {
  number: 42,
  state: 'Open',
  owner: 'example-org',
  repo: 'example-repo',
  title: 'Show delivery status',
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

const mobileMergeListDecorator: Decorator = (Story) => (
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
)

const otherPullRequests: PullRequest[] = [
  {
    number: 43,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-infra',
    title: 'Add retry settings',
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
    title: 'Record attempts',
    url: 'https://github.com/example-org/example-repo/pull/41',
    headSha: 'fixture-merged-sha',
    ciState: 'Unknown',
    isCurrentBranch: false,
  },
]

const allMergedPullRequests: PullRequest[] = [
  { ...currentBranchPullRequest, state: 'Merged' },
  ...otherPullRequests.map((pullRequest) => ({
    ...pullRequest,
    state: 'Merged' as const,
  })),
]

const meta = {
  component: SessionPullRequests,
  parameters: { layout: 'fullscreen' },
  args: {
    pullRequests: [currentBranchPullRequest],
    repo: 'example-org/example-repo',
    isMobile: false,
    open: false,
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
    pullRequests: allMergedPullRequests,
  },
}

export const AllPullRequestsAreMergedOnMobile: Story = {
  name: 'The mobile header hides the CI dot when all pull requests are merged.',
  args: {
    pullRequests: allMergedPullRequests,
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

export const MergeIsInProgressOnThePrButton: Story = {
  name: 'The PR button shows a spinner while a pull request is merging',
  args: {
    pullRequests: [mergeablePullRequest],
    mergeStatus: () => 'merging',
  },
}

export const PhoneMergeIsInProgressOnThePrButton: Story = {
  name: 'The phone PR button places the merge spinner on its corner',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    mergeStatus: () => 'merging',
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

export const MergeFailedOnThePrButton: Story = {
  name: 'The PR button marks a failed merge',
  args: {
    pullRequests: [mergeablePullRequest],
    mergeStatus: () => 'mergeFailed',
  },
}

export const PhoneMergeFailedOnThePrButton: Story = {
  name: 'The phone PR button places the failed merge mark on its corner',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    mergeStatus: () => 'mergeFailed',
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
  name: 'The desktop list shows status text beside each pull request',
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

export const ChecksHavePassedInTheDesktopList: Story = {
  name: 'The desktop list shows CI passed for a mergeable pull request',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
  },
  decorators: [desktopMergeButtonDecorator],
}

export const ChecksHavePassedInTheMobileList: Story = {
  name: 'The mobile sheet shows a merge button for a mergeable pull request',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
  },
  decorators: [mobileMergeListDecorator],
}

export const MergeIsInProgressInTheDesktopList: Story = {
  name: 'The desktop list disables Merge while a pull request is merging',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: () => 'merging',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MergeIsInProgressInTheMobileList: Story = {
  name: 'The mobile sheet disables Merge while a pull request is merging',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
    mergeStatus: () => 'merging',
  },
  decorators: [mobileMergeListDecorator],
}

export const MergeIsQueuedInTheMobileList: Story = {
  name: 'The mobile sheet shows a disabled queued merge action',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
    mergeStatus: (): AsyncActionStatus => 'queued',
  },
  decorators: [mobileMergeListDecorator],
}

export const MergeCanBeRetriedInTheDesktopList: Story = {
  name: 'The desktop list offers Retry after a merge fails',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'mergeFailed',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MergeCanBeRetriedInTheMobileList: Story = {
  name: 'The mobile sheet offers Retry after a merge fails',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
    mergeStatus: (): AsyncActionStatus => 'mergeFailed',
  },
  decorators: [mobileMergeListDecorator],
}

export const MobilePullRequestSheetIsOpen: Story = {
  name: 'The mobile sheet shows status text and Close at the bottom',
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

export const AFailingPullRequestCannotBeMergedOnMobile: Story = {
  name: 'The mobile sheet explains why a pull request cannot be merged',
  args: {
    pullRequests: [{ ...currentBranchPullRequest, ciState: 'Failed' }],
    isMobile: true,
    open: true,
  },
  decorators: [mobileMergeListDecorator],
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
  name: 'The desktop list reports a completed merge',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'merged',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const MergeSucceededInTheMobileList: Story = {
  name: 'The mobile sheet reports a completed merge',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
    mergeStatus: (): AsyncActionStatus => 'merged',
  },
  decorators: [mobileMergeListDecorator],
}

export const PullRequestWasAlreadyMergedInTheList: Story = {
  name: 'The desktop list reports that the pull request was already merged',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'alreadyMerged',
  },
  decorators: [desktopMergeButtonDecorator],
}

export const PullRequestWasAlreadyMergedInTheMobileList: Story = {
  name: 'The mobile sheet reports that the pull request was already merged',
  args: {
    pullRequests: [mergeablePullRequest],
    isMobile: true,
    open: true,
    mergeStatus: (): AsyncActionStatus => 'alreadyMerged',
  },
  decorators: [mobileMergeListDecorator],
}

export const MergeIsQueuedInTheList: Story = {
  name: 'The desktop list shows a disabled queued merge action',
  args: {
    pullRequests: [mergeablePullRequest],
    open: true,
    mergeStatus: (): AsyncActionStatus => 'queued',
  },
  decorators: [desktopMergeButtonDecorator],
}
