import type { Meta, StoryObj } from '@storybook/react-vite'
import { PanelRightOpen } from 'lucide-react'

import type { AsyncActionStatus } from '#async-action-feedback'
import {
  phoneTopbarCurrentPullRequest,
  phoneTopbarPullRequests,
} from '#components/phone-topbar-story-fixtures'
import { SessionPullRequestStatusBadge } from '#components/session-pull-requests'
import type { PullRequest } from '#types'

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
