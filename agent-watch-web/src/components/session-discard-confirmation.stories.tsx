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

const manyChanges = Array.from({ length: 7 }, (_, index) => ({
  status: 'M' as const,
  path: `src/module-${String(index + 1)}.ts`,
}))

const manyCommits = Array.from({ length: 6 }, (_, index) => ({
  sha: `c${String(index + 1)}d4e5f`,
  subject: `Update module ${String(index + 1)}`,
}))

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
  args: { changes: manyChanges, commits: manyCommits },
}

export const AllUncommittedChangesAreShown: Story = {
  name: 'The expanded file list shows all uncommitted changes.',
  args: { changes: manyChanges, defaultExpandedChanges: true },
}

export const AllCommitsAreShown: Story = {
  name: 'The expanded commit list shows all commits.',
  args: { commits: manyCommits, defaultExpandedCommits: true },
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

export const CommitsWithoutFileChanges: Story = {
  name: 'The dialog lists commits when no files changed',
  args: {
    changes: [],
    commits: [
      { sha: 'b2c3d4e', subject: 'Adjust the retry interval' },
      { sha: 'f5a6b7c', subject: 'Document the retry behavior' },
    ],
  },
}

export const IPhoneBottomSheet: Story = {
  name: 'The iPhone dialog appears as a bottom sheet',
  args: { mobile: true },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

export const IPhoneReviewAgainWithNewItems: Story = {
  name: 'The iPhone sheet asks for another review of new items',
  args: {
    mobile: true,
    reviewAgain: true,
    changes: [{ status: '??', path: 'tmp/retry-output.log', isNew: true }],
    commits: [
      { sha: 'd4e5f6a', subject: 'Add a retry timeout', isNew: true },
      { sha: 'b2c3d4e', subject: 'Adjust the retry interval' },
    ],
  },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

export const WorktreeWithoutABranch: Story = {
  name: 'The dialog explains that only the worktree will be deleted.',
  args: { branchName: null },
}
