import type { Meta, StoryObj } from '@storybook/react-vite'

import { PullRequestMergeButton } from '#components/pull-request-merge-button'

const meta = {
  component: PullRequestMergeButton,
  parameters: { layout: 'centered' },
  args: {
    status: 'ready',
    pullRequestNumber: 42,
    pullRequestRepo: 'example-org/service',
    isMobile: false,
    placement: 'row',
    onMerge: () => undefined,
    onRetry: () => undefined,
  },
  decorators: [
    (Story) => (
      <div
        style={{
          minWidth: 320,
          minHeight: 88,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'flex-end',
          padding: 20,
          background: 'var(--aw-bg-app)',
          color: 'var(--aw-text-primary)',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof PullRequestMergeButton>

export default meta
type Story = StoryObj<typeof meta>

export const ReadyRow: Story = {
  name: 'the desktop row shows an outline Merge action when checks pass',
}

export const ReadyMobileRow: Story = {
  name: 'the phone row keeps the Merge action easy to tap',
  args: { isMobile: true },
}

export const MergingRow: Story = {
  name: 'the row disables its action while the merge is running',
  args: { status: 'merging' },
}

export const QueuedRow: Story = {
  name: 'the row shows a disabled action while the merge is queued',
  args: { status: 'queued' },
}

export const FailedRow: Story = {
  name: 'the failed row offers Retry beside its failure label',
  args: { status: 'mergeFailed' },
}

export const UnavailableRow: Story = {
  name: 'the row explains why a pull request cannot be merged',
  args: { status: 'unavailable', unavailableReason: 'CI failing' },
}

export const MergedRow: Story = {
  name: 'the merged row shows its success status',
  args: { status: 'merged' },
}

export const AlreadyMergedRow: Story = {
  name: 'the row shows when the pull request was already merged',
  args: { status: 'alreadyMerged' },
}

export const ReadyBand: Story = {
  name: 'the merge band uses the filled Merge action',
  args: { placement: 'band' },
}
