import type { Meta, StoryObj } from '@storybook/react-vite'

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
  name: 'The header dims the pull request icon when all pull requests are merged',
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
  name: 'The desktop header shows the existing pull request list',
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

export const MobilePullRequestSheetIsOpen: Story = {
  name: 'The mobile header opens the pull request list as a bottom sheet',
  args: {
    pullRequests: [currentBranchPullRequest, ...otherPullRequests],
    isMobile: true,
    open: true,
    mobileAction: (
      <button className="conversation-pr-merge-button" type="button">
        Merge
      </button>
    ),
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
