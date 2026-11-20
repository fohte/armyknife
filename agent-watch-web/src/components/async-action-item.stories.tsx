import type { Meta, StoryObj } from '@storybook/react-vite'

import { AsyncActionItem } from '#components/async-action-item'

const meta = {
  component: AsyncActionItem,
  args: {
    action: {
      id: 1,
      kind: 'merge',
      sessionId: 'sample-session',
      sessionTitle: 'Review the deployment plan',
      pullRequestNumber: 42,
      status: 'merging',
      createdAt: 1_800_000_000_000,
    },
    form: 'toast',
  },
} satisfies Meta<typeof AsyncActionItem>

export default meta
type Story = StoryObj<typeof meta>

export const MergeIsRunning: Story = {
  name: 'The merge is running in a toast.',
}

export const MergeSucceededInActivity: Story = {
  name: 'The merge result appears in the Activity list.',
  args: {
    form: 'row',
    action: { ...meta.args.action, status: 'merged' },
  },
}

export const PullRequestWasAlreadyMerged: Story = {
  name: 'The toast reports that the pull request was already merged.',
  args: { action: { ...meta.args.action, status: 'alreadyMerged' } },
}

export const MergeWasAddedToQueue: Story = {
  name: 'The queued merge appears in the Activity list.',
  args: {
    form: 'row',
    action: {
      ...meta.args.action,
      status: 'queued',
      message: 'It merges when the queue’s checks pass.',
    },
  },
}

export const MergeFailureCanBeRetried: Story = {
  name: 'The failed merge offers Retry and Details.',
  args: {
    action: {
      ...meta.args.action,
      status: 'mergeFailed',
      error: 'The branch changed after confirmation.',
    },
  },
}

export const CloseIsRunningInConversation: Story = {
  name: 'The conversation shows a running close action.',
  args: {
    form: 'line',
    action: {
      id: 1,
      sessionId: 'sample-session',
      sessionTitle: 'Review the deployment plan',
      kind: 'close',
      status: 'closing',
      createdAt: 1_800_000_000_000,
    },
  },
}

export const CloseSucceededInActivity: Story = {
  name: 'A successful close remains in Activity.',
  args: {
    form: 'row',
    action: {
      id: 2,
      sessionId: 'sample-session',
      sessionTitle: 'Review the deployment plan',
      kind: 'close',
      status: 'closed',
      createdAt: 1_800_000_000_000,
    },
  },
}

export const CloseFailedAndCanBeRetried: Story = {
  name: 'A failed close shows its reason and Retry action.',
  args: {
    action: {
      id: 2,
      sessionId: 'sample-session',
      sessionTitle: 'Review the deployment plan',
      kind: 'close',
      status: 'closeFailed',
      error: 'The session could not be closed.',
      createdAt: 1_800_000_000_000,
    },
  },
}
