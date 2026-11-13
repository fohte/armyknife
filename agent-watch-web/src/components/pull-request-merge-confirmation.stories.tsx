import type { Meta, StoryObj } from '@storybook/react-vite'

import { PullRequestMergeConfirmation } from '#components/pull-request-merge-confirmation'

const meta = {
  component: PullRequestMergeConfirmation,
  args: {
    number: 42,
    headSha: 'confirmed-example-head',
    merging: false,
    onCancel: () => undefined,
    onConfirm: () => undefined,
  },
} satisfies Meta<typeof PullRequestMergeConfirmation>

export default meta
type Story = StoryObj<typeof meta>

export const MergeIsReadyForConfirmation: Story = {
  name: 'The merge confirmation is ready for approval.',
}

export const MergeRequestIsSending: Story = {
  name: 'The merge confirmation blocks input while the request is sending.',
  args: { merging: true },
}
