import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionDiscardConfirmation } from '#components/session-discard-confirmation'

const args = {
  sessionName: 'Revise the notification retry flow',
  branchName: 'feature/retry-limit',
  worktreePath: '~/src/example/service/.worktrees/retry-limit',
  changes: [
    { status: 'M' as const, path: 'src/retry.ts' },
    { status: '??' as const, path: 'tmp/retry-debug.log' },
  ],
  commits: [
    { sha: 'a1b2c3d', subject: 'Expose the retry limit as a setting' },
    { sha: 'e4f5a6b', subject: 'Show a warning after the limit is reached' },
  ],
  onCancel: () => undefined,
  onDiscard: () => undefined,
}

const meta = {
  component: SessionDiscardConfirmation,
  args,
} satisfies Meta<typeof SessionDiscardConfirmation>

export default meta
type Story = StoryObj<typeof meta>

export const AShortListOfChanges: Story = {
  name: 'The dialog lists a few changes and commits',
}

export const MoreThanFiveItems: Story = {
  name: 'The dialog offers to show the remaining items',
  args: {
    changes: Array.from({ length: 7 }, (_, index) => ({
      status: 'M' as const,
      path: `src/module-${String(index + 1)}.ts`,
    })),
    commits: Array.from({ length: 6 }, (_, index) => ({
      sha: `c${String(index + 1)}d4e5f`,
      subject: `Update module ${String(index + 1)}`,
    })),
  },
}

export const ReviewAgainWithNewItems: Story = {
  name: 'The dialog highlights changes that appeared after confirmation',
  args: {
    reviewAgain: true,
    changes: [
      { status: '??', path: 'tmp/new-output.log', isNew: true },
      { status: 'M', path: 'src/retry.ts' },
    ],
    commits: [
      { sha: 'f7a8b9c', subject: 'Add a retry delay', isNew: true },
      { sha: 'a1b2c3d', subject: 'Expose the retry limit as a setting' },
    ],
  },
}

export const IPhoneBottomSheet: Story = {
  name: 'The iPhone dialog appears as a bottom sheet',
  args: { mobile: true },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}
