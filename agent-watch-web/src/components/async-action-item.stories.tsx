import '#components/async-action-feedback-host.css'

import type { Meta, StoryObj } from '@storybook/react-vite'

import type { AsyncAction } from '#async-action-feedback'
import { AsyncActionItem } from '#components/async-action-item'
import { BangCommandButton } from '#components/bang-command-button'
import { UserWithContent } from '#components/user-with-content'
import type { ChatItem } from '#types'

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
      context.id.includes('rewind') ? (
        <Story />
      ) : context.args.form === 'toast' ? (
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

type FeedbackComparisonState = 'running' | 'succeeded' | 'failed'

function RewindCloseComparison({ state }: { state: FeedbackComparisonState }) {
  const rewindStatus =
    state === 'running'
      ? 'rewinding'
      : state === 'succeeded'
        ? 'rewound'
        : 'rewindFailed'
  const closeStatus =
    state === 'running'
      ? 'closing'
      : state === 'succeeded'
        ? 'closed'
        : 'closeFailed'
  const renderItem = (
    kind: 'rewind' | 'close',
    status: typeof rewindStatus | typeof closeStatus,
  ) => (
    <div>
      <h3>{kind === 'rewind' ? 'Rewind' : 'Close'}</h3>
      <div
        className="async-action-toast-stack"
        style={{
          position: 'static',
          width: '100%',
          maxHeight: 'none',
          flexDirection: 'column',
          overflow: 'visible',
          pointerEvents: 'auto',
        }}
      >
        <AsyncActionItem
          action={{
            id: kind === 'rewind' ? 1 : 2,
            kind,
            sessionId: 'example-session',
            sessionTitle: 'Review the example request',
            status,
            ...(state === 'failed'
              ? {
                  error: `The example session could not be ${kind === 'rewind' ? 'rewound' : 'closed'}.`,
                }
              : {}),
            createdAt: 1_800_000_000_000,
          }}
          form="toast"
          onRetry={() => undefined}
          onDetails={() => undefined}
        />
      </div>
    </div>
  )

  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
        gap: 24,
        maxWidth: 900,
        padding: 32,
      }}
    >
      {renderItem('rewind', rewindStatus)}
      {renderItem('close', closeStatus)}
    </div>
  )
}

function RewindCloseAllStatesComparison() {
  return (
    <div style={{ display: 'grid', gap: 24, padding: 32 }}>
      {(['running', 'succeeded', 'failed'] as const).map((state) => (
        <section key={state}>
          <h2>
            {state === 'running'
              ? 'Running'
              : state === 'succeeded'
                ? 'Succeeded'
                : 'Failed'}
          </h2>
          <RewindCloseComparison state={state} />
        </section>
      ))}
    </div>
  )
}

export const MergeIsRunning: Story = {
  name: 'The merge is running in a toast.',
}

export const RewindIsRunningBesideClose: Story = {
  name: 'The running rewind matches the running close toast.',
  render: () => <RewindCloseComparison state="running" />,
}

export const RewindSucceededBesideClose: Story = {
  name: 'The successful rewind matches the successful close toast.',
  render: () => <RewindCloseComparison state="succeeded" />,
}

export const RewindFailedBesideClose: Story = {
  name: 'The failed rewind avoids replaying an ambiguous operation.',
  render: () => <RewindCloseComparison state="failed" />,
}

export const RewindAndCloseFeedbackComparison: Story = {
  name: 'Rewind and Close feedback is compared across each outcome.',
  render: () => <RewindCloseAllStatesComparison />,
}

export const MergeSucceededInActivity: Story = {
  name: 'The merge result appears in the Activity list.',
  args: {
    form: 'row',
    action: { ...meta.args.action, status: 'merged' },
  },
}

const launchAction: AsyncAction = {
  id: 6,
  kind: 'launch',
  sessionId: 'example-launch-session',
  sessionTitle: 'Review the example rollout',
  initialMessage: 'Review the rollout plan.\nCompare the fallback options.',
  repo: 'example-org/example-service',
  status: 'launching',
  createdAt: 1_800_000_000_000,
}

export const SessionIsStarting: Story = {
  name: 'The toast shows the first message and repository while starting.',
  args: { form: 'toast', action: launchAction },
}

export const SessionStartedInActivity: Story = {
  name: 'Activity keeps a started session with an Open action.',
  args: {
    form: 'row',
    action: { ...launchAction, status: 'launched' },
    onOpenSession: () => undefined,
  },
}

export const SessionStartFailed: Story = {
  name: 'The failed launch toast shows its reason, Retry, and Details.',
  args: {
    form: 'toast',
    action: {
      ...launchAction,
      status: 'launchFailed',
      error: 'The example session could not be started.',
    },
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
  discardable: true,
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
        changes: [
          { status: 'M', path: 'src/example.ts' },
          { status: '??', path: 'src/example.test.ts' },
        ],
        commits: [],
        branch: 'feature/example-work',
        merged: true,
        reason: null,
        processes: [],
        commitInspectionUnavailable: false,
        processInspectionUnavailable: false,
      },
    },
  },
}

export const NewChangesRequireAnotherReview: Story = {
  name: 'The changed worktree can be reviewed again before discarding.',
  args: {
    form: 'row',
    action: {
      id: 9,
      sessionId: 'example-session',
      sessionTitle: 'Review the example configuration',
      kind: 'discard',
      status: 'deleteChanged',
      message: 'Changed after you confirmed: 1 new file and 1 new commit.',
      checks: {
        path: '/worktrees/example-project',
        files: ['src/example.ts', 'tmp/example-output.log'],
        changes: [
          { status: 'M', path: 'src/example.ts' },
          { status: '??', path: 'tmp/example-output.log' },
        ],
        commits: [
          {
            sha: '2222222222222222222222222222222222222222',
            subject: 'add example output',
          },
        ],
        branch: 'feature/example-work',
        merged: true,
        reason: null,
        processes: [],
        commitInspectionUnavailable: false,
        processInspectionUnavailable: false,
      },
      createdAt: 1_800_000_000_000,
    },
    onReviewDiscardAgain: () => undefined,
    onCloseKeepWorktree: () => undefined,
  },
}

export const RunningProcessBlocksDiscard: Story = {
  name: 'A running process blocks discard and can be retried.',
  args: {
    action: {
      id: 10,
      sessionId: 'example-session',
      sessionTitle: 'Review the example configuration',
      kind: 'discard',
      status: 'deleteBlocked',
      message:
        'Running in the worktree: 12345 (example-server). Stop it, then Retry.',
      error: 'The worktree still has a running process.',
      checks: {
        path: '/worktrees/example-project',
        files: ['src/example.ts'],
        changes: [{ status: 'M', path: 'src/example.ts' }],
        commits: [],
        branch: 'feature/example-work',
        merged: true,
        reason: null,
        processes: [{ pid: 12345, name: 'example-server' }],
        commitInspectionUnavailable: false,
        processInspectionUnavailable: false,
      },
      createdAt: 1_800_000_000_000,
    },
    onRetry: () => undefined,
    onOpenInTerminal: () => undefined,
    onCloseKeepWorktree: () => undefined,
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
        changes: [],
        commits: [
          {
            sha: '1111111111111111111111111111111111111111',
            subject: 'add example implementation',
          },
        ],
        branch: 'feature/example-work',
        merged: false,
        reason: 'The example pull request is still open.',
        processes: [],
        commitInspectionUnavailable: false,
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
        changes: [],
        commits: [],
        branch: 'feature/example-work',
        merged: true,
        reason: null,
        processes: [
          { pid: 12345, name: 'example-server' },
          { pid: 23456, name: 'example-watcher' },
        ],
        commitInspectionUnavailable: false,
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
        changes: [{ status: 'M', path: 'src/example.ts' }],
        commits: [
          {
            sha: '1111111111111111111111111111111111111111',
            subject: 'add example implementation',
          },
        ],
        branch: 'feature/example-work',
        merged: false,
        reason: 'The example pull request is still open.',
        processes: [{ pid: 12345, name: 'example-server' }],
        commitInspectionUnavailable: false,
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
        changes: [],
        commits: [],
        branch: null,
        merged: null,
        reason: null,
        processes: [],
        commitInspectionUnavailable: false,
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

const reviewDocumentAction = {
  id: 5,
  sessionId: 'sample-session',
  sessionTitle: 'Review the example request',
  kind: 'review_document' as const,
  reviewDocumentAction: 'Post comment' as const,
  status: 'reviewDocumentActionRunning' as const,
  createdAt: 1_800_000_000_000,
}

export const ReviewDocumentPostIsInProgress: Story = {
  name: 'Posting an issue comment shows an in-progress action',
  args: { form: 'toast', action: reviewDocumentAction },
}

export const ReviewDocumentPostSucceeded: Story = {
  name: 'A posted issue comment shows a success action',
  args: {
    form: 'toast',
    action: {
      ...reviewDocumentAction,
      status: 'reviewDocumentActionSucceeded',
    },
  },
}

export const ReviewDocumentPostFailed: Story = {
  name: 'A failed issue comment post can be retried',
  args: {
    form: 'toast',
    action: {
      ...reviewDocumentAction,
      status: 'reviewDocumentActionFailed',
      error: 'The remote issue could not be updated.',
    },
  },
}

const reviewDocumentUpdateAction = {
  ...reviewDocumentAction,
  reviewDocumentAction: 'Update PR #73',
}

export const ReviewDocumentPrUpdateIsInProgress: Story = {
  name: 'Updating an existing pull request shows its number',
  args: { form: 'toast', action: reviewDocumentUpdateAction },
}

export const ReviewDocumentPrUpdateFailed: Story = {
  name: 'A failed pull request update names the existing pull request',
  args: {
    form: 'toast',
    action: {
      ...reviewDocumentUpdateAction,
      status: 'reviewDocumentActionFailed',
      error: 'The existing pull request could not be updated.',
    },
  },
}

type RunCommandSceneState = 'running' | 'succeeded' | 'failed' | 'timedOut'

function RunCommandFeedbackScene({
  state,
  mobile,
  outputHighlighted = false,
}: {
  state: RunCommandSceneState
  mobile: boolean
  outputHighlighted?: boolean
}) {
  const commandStatus =
    state === 'running'
      ? 'running'
      : state === 'succeeded'
        ? 'succeeded'
        : 'failed'
  const commandButtonLabel =
    state === 'succeeded'
      ? 'Succeeded'
      : state === 'failed' || state === 'timedOut'
        ? 'Failed'
        : 'Run command'
  const commandMessage =
    state === 'succeeded'
      ? 'Completed'
      : state === 'failed'
        ? 'Failed (exit code 1)'
        : state === 'timedOut'
          ? 'Timed out'
          : undefined
  const action: AsyncAction = {
    ...commandAction,
    createdAt: new Date('2030-09-12T03:03:00.000Z').getTime(),
    status:
      state === 'running'
        ? 'runningCommand'
        : state === 'succeeded'
          ? 'ranCommand'
          : state === 'failed'
            ? 'commandFailed'
            : 'commandTimedOut',
    ...(state === 'failed' ? { exitCode: 1 } : {}),
  }
  const resultMessage: Extract<ChatItem, { type: 'user' }> | null =
    state === 'running'
      ? null
      : {
          type: 'user',
          timestamp: '2030-09-12T03:03:00.000Z',
          text: `${state === 'timedOut' ? 'Command timed out after 30 seconds' : `Command exited with code ${state === 'succeeded' ? '0' : '1'}`} in /workspace/example.

Command:
\`\`\`text
$ printf 'sample output'
\`\`\`

stdout:
\`\`\`text
${state === 'succeeded' ? 'sample output' : '(empty)'}
\`\`\`

stderr:
\`\`\`text
${state === 'failed' ? 'sample error' : '(empty)'}
\`\`\``,
        }
  const time = '12:03'

  return (
    <div
      style={{
        boxSizing: 'border-box',
        minHeight: '100vh',
        padding: mobile ? '20px 16px 140px' : '48px 32px 160px',
        background: '#0a0a0a',
        color: 'var(--aw-text-body)',
      }}
    >
      <main
        style={{
          boxSizing: 'border-box',
          display: 'flex',
          width: '100%',
          maxWidth: 760,
          minHeight: mobile ? 'calc(100vh - 160px)' : 'calc(100vh - 208px)',
          flexDirection: 'column',
          gap: 18,
          margin: '0 auto',
          padding: mobile ? 0 : 24,
          border: mobile ? '0' : '1px solid var(--aw-border-control)',
          background: '#0f0f0f',
        }}
      >
        <header
          style={{
            borderBottom: '1px solid var(--aw-border-control)',
            paddingBottom: 12,
            color: 'var(--aw-text-primary)',
            fontSize: 13,
          }}
        >
          example/web-app · Review the command output
        </header>
        <section
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 10,
            padding: mobile ? 12 : 16,
            border: '1px solid var(--aw-border-control)',
            background: 'var(--aw-bg-popover)',
          }}
        >
          <p style={{ margin: 0, color: 'var(--aw-text-body)', fontSize: 13 }}>
            Run the command to check the example output.
          </p>
          <pre
            style={{
              margin: 0,
              padding: 12,
              overflowWrap: 'anywhere',
              background: 'var(--aw-bg-palette-muted)',
              color: 'var(--aw-text-primary)',
              fontFamily: 'var(--font-mono, monospace)',
              fontSize: 12,
              whiteSpace: 'pre-wrap',
            }}
          >
            ! printf 'sample output'
          </pre>
          <div
            className="conversation-pane"
            data-mobile={mobile ? 'true' : undefined}
          >
            <BangCommandButton
              label={commandButtonLabel}
              status={commandStatus}
              {...(commandMessage === undefined
                ? {}
                : { message: commandMessage })}
              busy={state === 'running'}
              onClick={() => undefined}
            />
          </div>
        </section>
        {resultMessage !== null ? (
          <UserWithContent
            message={resultMessage}
            now={new Date('2030-09-12T03:03:00.000Z')}
            commandOutputHighlighted={outputHighlighted}
          />
        ) : null}
        <div style={{ marginTop: 'auto', paddingTop: 4 }}>
          <AsyncActionItem action={action} form="line" time={time} />
        </div>
        <div
          style={{
            minHeight: 42,
            border: '1px solid var(--aw-border-control)',
            padding: '12px 14px',
            color: 'var(--aw-text-muted)',
            fontSize: 12,
          }}
        >
          Message the session
        </div>
      </main>
      {!outputHighlighted ? (
        <div className="async-action-toast-stack">
          <AsyncActionItem
            action={action}
            form="toast"
            onDismiss={() => undefined}
            onShowOutput={() => undefined}
          />
        </div>
      ) : null}
    </div>
  )
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
  args: { form: 'line', action: commandAction, time: '12:03' },
}

export const RunCommandSucceededInConversation: Story = {
  name: 'The conversation marks the command as successful.',
  args: {
    form: 'line',
    action: { ...commandAction, status: 'ranCommand' },
    time: '12:03',
  },
}

export const RunCommandFailedInConversation: Story = {
  name: 'The conversation shows the failed command result.',
  args: {
    form: 'line',
    action: { ...commandAction, status: 'commandFailed', exitCode: 1 },
    time: '12:03',
  },
}

export const RunCommandTimedOutInConversation: Story = {
  name: 'The conversation shows the timed out command result.',
  args: {
    form: 'line',
    action: { ...commandAction, status: 'commandTimedOut' },
    time: '12:03',
  },
}

function commandSceneStory(
  state: RunCommandSceneState,
  mobile: boolean,
  outputHighlighted = false,
): Story {
  return {
    args: { form: 'row' },
    ...(mobile
      ? { parameters: { viewport: { defaultViewport: 'mobile2' } } }
      : {}),
    render: () => (
      <RunCommandFeedbackScene
        state={state}
        mobile={mobile}
        outputHighlighted={outputHighlighted}
      />
    ),
  }
}

export const RunCommandRunningSceneOnPc: Story = {
  name: 'The PC conversation shows a running command and its toast.',
  ...commandSceneStory('running', false),
}

export const RunCommandRunningSceneOnIPhone: Story = {
  name: 'The iPhone conversation shows a running command and its toast.',
  ...commandSceneStory('running', true),
}

export const RunCommandSucceededSceneOnPc: Story = {
  name: 'The PC conversation shows a successful command and its toast.',
  ...commandSceneStory('succeeded', false),
}

export const RunCommandSucceededSceneOnIPhone: Story = {
  name: 'The iPhone conversation shows a successful command and its toast.',
  ...commandSceneStory('succeeded', true),
}

export const RunCommandFailedSceneOnPc: Story = {
  name: 'The PC conversation shows a failed command and its toast.',
  ...commandSceneStory('failed', false),
}

export const RunCommandFailedSceneOnIPhone: Story = {
  name: 'The iPhone conversation shows a failed command and its toast.',
  ...commandSceneStory('failed', true),
}

export const RunCommandTimedOutSceneOnPc: Story = {
  name: 'The PC conversation shows a timed out command and its toast.',
  ...commandSceneStory('timedOut', false),
}

export const RunCommandTimedOutSceneOnIPhone: Story = {
  name: 'The iPhone conversation shows a timed out command and its toast.',
  ...commandSceneStory('timedOut', true),
}

export const FailedCommandOutputIsHighlightedOnPc: Story = {
  name: 'The PC conversation highlights a failed result after Show output.',
  ...commandSceneStory('failed', false, true),
}

export const FailedCommandOutputIsHighlightedOnIPhone: Story = {
  name: 'The iPhone conversation highlights a failed result after Show output.',
  ...commandSceneStory('failed', true, true),
}

export const TimedOutCommandOutputIsHighlightedOnPc: Story = {
  name: 'The PC conversation highlights a timed out result after Show output.',
  ...commandSceneStory('timedOut', false, true),
}

export const TimedOutCommandOutputIsHighlightedOnIPhone: Story = {
  name: 'The iPhone conversation highlights a timed out result after Show output.',
  ...commandSceneStory('timedOut', true, true),
}
