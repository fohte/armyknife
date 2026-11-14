import type { Meta, StoryObj } from '@storybook/react-vite'

import { PullRequestMergeAction } from '#components/pull-request-merge-action'

const meta = {
  component: PullRequestMergeAction,
  args: {
    merging: false,
    onClick: () => undefined,
  },
} satisfies Meta<typeof PullRequestMergeAction>

export default meta
type Story = StoryObj<typeof meta>

export const MergeIsAvailable: Story = {
  name: 'The merge action is available.',
}

export const MergeIsInProgress: Story = {
  name: 'The merge action shows that a merge is in progress.',
  args: { merging: true },
}
