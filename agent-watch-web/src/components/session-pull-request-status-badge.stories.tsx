import type { Meta, StoryObj } from '@storybook/react-vite'
import { PanelRightOpen } from 'lucide-react'

import type { AsyncActionStatus } from '#async-action-feedback'
import { SessionPullRequestStatusBadge } from '#components/session-pull-requests'
import type { PullRequest } from '#types'

const pullRequests = [
  {
    number: 2050,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-service',
    ciState: 'InProgress',
    isCurrentBranch: true,
    headSha: 'sample-phone-pr-head-sha',
  },
  {
    number: 2051,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-service',
    ciState: 'Successful',
    isCurrentBranch: false,
    headSha: 'sample-phone-pr-head-sha-2',
  },
  {
    number: 2052,
    state: 'Open',
    owner: 'example-team',
    repo: 'sample-infra',
    ciState: 'Successful',
    isCurrentBranch: false,
    headSha: 'sample-phone-pr-head-sha-3',
  },
] as const
const noMerge: (pullRequest: PullRequest) => AsyncActionStatus | null = () =>
  null

const meta = {
  component: SessionPullRequestStatusBadge,
  args: {
    pullRequests,
    mergeStatus: noMerge,
  },
  decorators: [
    (Story, context) => {
      const status = context.args.mergeStatus(mergeablePullRequest)
      const hasMergeFailure = status === 'mergeFailed'

      return (
        <div
          className="conversation-pane"
          data-mobile="true"
          style={{ padding: 8 }}
        >
          <button
            className={`mobile-conversation-status-toggle${hasMergeFailure ? ' is-merge-failed' : ''}`}
            type="button"
            aria-label="Open session status panel"
          >
            <PanelRightOpen aria-hidden="true" size={18} />
            <Story />
          </button>
        </div>
      )
    },
  ],
} satisfies Meta<typeof SessionPullRequestStatusBadge>

export default meta
type Story = StoryObj<typeof meta>

const mergeablePullRequest: PullRequest = {
  number: 2050,
  state: 'Open',
  owner: 'example-org',
  repo: 'example-service',
  ciState: 'Successful',
  isCurrentBranch: true,
  headSha: 'sample-phone-pr-head-sha',
}

export const MergeIsInProgress: Story = {
  name: 'The phone status button shows an active merge',
  args: { mergeStatus: (): AsyncActionStatus => 'merging' },
}

export const CiIsRunning: Story = {
  name: 'The phone status button shows running CI and the PR count',
  args: { mergeStatus: noMerge },
}

export const MergeHasFailed: Story = {
  name: 'The phone status button marks a failed merge',
  args: { mergeStatus: (): AsyncActionStatus => 'mergeFailed' },
}
