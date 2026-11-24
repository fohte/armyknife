import type { Meta, StoryObj } from '@storybook/react-vite'
import { PanelRightOpen } from 'lucide-react'

import type { AsyncActionStatus } from '#async-action-feedback'
import { SessionPullRequestStatusBadge } from '#components/session-pull-requests'
import type { PullRequest } from '#types'

const phoneTopbarCurrentPullRequest: PullRequest = {
  number: 2050,
  state: 'Open',
  owner: 'example',
  repo: 'service',
  title: '再送処理を retry() にまとめる',
  url: 'https://github.com/example/service/pull/2050',
  headSha: 'sample-phone-pr-head-sha',
  ciState: 'InProgress',
  isCurrentBranch: true,
}

const phoneTopbarPullRequests: PullRequest[] = [
  phoneTopbarCurrentPullRequest,
  {
    number: 64,
    state: 'Open',
    owner: 'example',
    repo: 'infra',
    title: '再送キューの上限を設定に出す',
    url: 'https://github.com/example/infra/pull/64',
    headSha: 'sample-phone-pr-head-sha-2',
    ciState: 'Successful',
    isCurrentBranch: false,
  },
  {
    number: 2041,
    state: 'Merged',
    owner: 'example',
    repo: 'service',
    title: '再送のログを整える',
    url: 'https://github.com/example/service/pull/2041',
    headSha: 'sample-phone-pr-head-sha-3',
    ciState: 'Successful',
    isCurrentBranch: false,
  },
]

const noMerge: (pullRequest: PullRequest) => AsyncActionStatus | null = () =>
  null

const meta = {
  component: SessionPullRequestStatusBadge,
  args: {
    pullRequests: phoneTopbarPullRequests,
    mergeStatus: noMerge,
  },
  decorators: [
    (Story, context) => {
      const status = context.args.mergeStatus(phoneTopbarCurrentPullRequest)
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
