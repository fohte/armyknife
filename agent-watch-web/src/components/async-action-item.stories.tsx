import '#components/async-action-feedback-host.css'

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
  decorators: [
    (Story, context) =>
      context.args.form === 'toast' ? (
        <div className="async-action-toast-stack">
          <Story />
        </div>
      ) : (
        <Story />
      ),
  ],
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

export const QueuedMergeToastOnIPhone: Story = {
  name: 'The queued merge appears in an iPhone toast.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    form: 'toast',
    action: {
      ...meta.args.action,
      status: 'queued',
      message: 'It merges when the queue’s checks pass.',
    },
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          padding: 16,
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const QueuedMergeLineOnIPhone: Story = {
  name: 'The conversation marks the merge as queued on an iPhone.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    form: 'line',
    action: {
      ...meta.args.action,
      status: 'queued',
      message: 'It merges when the queue’s checks pass.',
    },
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          padding: 16,
        }}
      >
        <Story />
      </div>
    ),
  ],
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

export const FailedMergeToastOnIPhone: Story = {
  name: 'An iPhone merge toast shows its failure reason and actions.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    form: 'toast',
    action: {
      ...meta.args.action,
      status: 'mergeFailed',
      error: 'The example branch changed after confirmation.',
    },
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          padding: 16,
        }}
      >
        <Story />
      </div>
    ),
  ],
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

export const ClosedSessionToastOnIPhone: Story = {
  name: 'A closed session result appears in an iPhone toast.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    form: 'toast',
    action: {
      id: 3,
      sessionId: 'example-session',
      sessionTitle: 'Review the example configuration',
      kind: 'close',
      status: 'closed',
      createdAt: 1_800_000_000_000,
    },
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          padding: 16,
        }}
      >
        <Story />
      </div>
    ),
  ],
}

export const CloseFailedAndCanBeRetried: Story = {
  name: 'A close failure without checks shows its message and Retry.',
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

const failedCloseInActivity = {
  id: 3,
  sessionId: 'example-session',
  sessionTitle: 'Review the example configuration',
  kind: 'close' as const,
  status: 'closeFailed' as const,
  error: 'The session could not be closed.',
  createdAt: 1_800_000_000_000,
}

export const UncommittedFilesAreListed: Story = {
  name: 'Activity lists every uncommitted file.',
  args: {
    form: 'row',
    action: {
      ...failedCloseInActivity,
      checks: {
        path: '/worktrees/example-project',
        files: ['src/example.ts', 'src/example.test.ts'],
        branch: 'feature/example-work',
        merged: true,
        reason: null,
        processes: [],
        processInspectionUnavailable: false,
      },
    },
  },
}

export const UnmergedBranchShowsItsReason: Story = {
  name: 'Activity shows the unmerged branch and reason.',
  args: {
    form: 'row',
    action: {
      ...failedCloseInActivity,
      checks: {
        path: '/worktrees/example-project',
        files: [],
        branch: 'feature/example-work',
        merged: false,
        reason: 'The example pull request is still open.',
        processes: [],
        processInspectionUnavailable: false,
      },
    },
  },
}

export const RunningProcessesAreListed: Story = {
  name: 'Activity lists each running process.',
  args: {
    form: 'row',
    action: {
      ...failedCloseInActivity,
      checks: {
        path: '/worktrees/example-project',
        files: [],
        branch: 'feature/example-work',
        merged: true,
        reason: null,
        processes: [
          { pid: 12345, name: 'example-server' },
          { pid: 23456, name: 'example-watcher' },
        ],
        processInspectionUnavailable: false,
      },
    },
  },
}

export const AllThreeChecksAreVisible: Story = {
  name: 'Activity shows all three checks together.',
  args: {
    form: 'row',
    action: {
      ...failedCloseInActivity,
      checks: {
        path: '/worktrees/example-project',
        files: ['src/example.ts'],
        branch: 'feature/example-work',
        merged: false,
        reason: 'The example pull request is still open.',
        processes: [{ pid: 12345, name: 'example-server' }],
        processInspectionUnavailable: false,
      },
    },
  },
}

export const ProcessCheckIsUnavailable: Story = {
  name: 'Activity marks an unavailable process check.',
  args: {
    form: 'row',
    action: {
      ...failedCloseInActivity,
      checks: {
        path: '/worktrees/example-project',
        files: [],
        branch: null,
        merged: null,
        reason: null,
        processes: [],
        processInspectionUnavailable: true,
      },
    },
  },
}

const commandAction = {
  id: 4,
  sessionId: 'example-session',
  sessionTitle: 'Review the example output',
  kind: 'command' as const,
  command: "printf 'sample output'",
  status: 'runningCommand' as const,
  createdAt: 1_800_000_000_000,
}

export const RunCommandIsRunning: Story = {
  name: 'A running command appears in a toast.',
  args: { form: 'toast', action: commandAction },
}

export const RunningRunCommandInActivity: Story = {
  name: 'Activity lists a running command.',
  args: {
    form: 'row',
    action: commandAction,
  },
}

export const RunCommandSucceeded: Story = {
  name: 'A successful command appears in a toast.',
  args: {
    form: 'toast',
    action: { ...commandAction, status: 'ranCommand' },
  },
}

export const SuccessfulRunCommandInActivity: Story = {
  name: 'Activity keeps a successful command result.',
  args: {
    form: 'row',
    action: { ...commandAction, status: 'ranCommand' },
  },
}

export const RunCommandFailed: Story = {
  name: 'A failed command offers its output in a toast.',
  args: {
    form: 'toast',
    action: { ...commandAction, status: 'commandFailed', exitCode: 1 },
  },
}

export const FailedRunCommandInActivity: Story = {
  name: 'Activity offers output for a failed command.',
  args: {
    form: 'row',
    action: { ...commandAction, status: 'commandFailed', exitCode: 1 },
  },
}

export const RunCommandTimedOut: Story = {
  name: 'A timed out command explains when it stopped.',
  args: {
    form: 'toast',
    action: { ...commandAction, status: 'commandTimedOut' },
  },
}

export const TimedOutRunCommandInActivity: Story = {
  name: 'Activity explains when a command timed out.',
  args: {
    form: 'row',
    action: { ...commandAction, status: 'commandTimedOut' },
  },
}

export const RunCommandIsRunningInConversation: Story = {
  name: 'The conversation shows a running command.',
  args: { form: 'line', action: commandAction },
}

export const RunCommandSucceededInConversation: Story = {
  name: 'The conversation marks the command as successful.',
  args: {
    form: 'line',
    action: { ...commandAction, status: 'ranCommand' },
  },
}

export const RunCommandFailedInConversation: Story = {
  name: 'The conversation shows the failed command result.',
  args: {
    form: 'line',
    action: { ...commandAction, status: 'commandFailed', exitCode: 1 },
  },
}

export const RunCommandTimedOutInConversation: Story = {
  name: 'The conversation shows the timed out command result.',
  args: {
    form: 'line',
    action: { ...commandAction, status: 'commandTimedOut' },
  },
}
