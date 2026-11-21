import type { Meta, StoryObj } from '@storybook/react-vite'

import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'

const meta = {
  component: AsyncActionFeedbackHost,
  args: {
    actions: [
      {
        id: 1,
        kind: 'merge',
        sessionId: 'sample-session',
        sessionTitle: 'Review the deployment plan',
        pullRequestNumber: 42,
        status: 'mergeFailed',
        error: 'The branch changed after confirmation.',
        createdAt: 1_800_000_000_000,
      },
    ],
    toastIds: [1],
    activityOpen: false,
    toastBottomOffset: null,
    selectedActionId: null,
    retry: () => undefined,
    dismissToast: () => undefined,
    clearFinished: () => undefined,
    openActivity: () => undefined,
    closeActivity: () => undefined,
    closeWithWorktree: () => undefined,
    openInTerminal: () => undefined,
  },
} satisfies Meta<typeof AsyncActionFeedbackHost>

export default meta
type Story = StoryObj<typeof meta>

export const FailureToastIsVisible: Story = {
  name: 'A failed merge stays visible as a toast.',
}

export const ActivityKeepsTheResult: Story = {
  name: 'Activity lists results and can retry failures.',
  args: { activityOpen: true, toastIds: [] },
}

export const ActivityShowsEmptyState: Story = {
  name: 'Activity explains where merge and close results will appear.',
  args: { actions: [], toastIds: [], activityOpen: true },
}

export const PhoneShowsEmptyActivity: Story = {
  name: 'The phone shows the empty Activity state.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: { actions: [], toastIds: [], activityOpen: true },
}

export const ToastsSitAboveAnOpenPhoneSheet: Story = {
  name: 'The phone action toast sits above an open sheet.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    toastBottomOffset: 360,
    actions: [
      {
        id: 1,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example deployment',
        pullRequestNumber: 42,
        status: 'queued',
        message: 'It merges when the queue checks pass.',
        createdAt: 1_800_000_000_000,
      },
    ],
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          height: 844,
        }}
      >
        <div
          aria-label="Open pull request sheet"
          style={{
            position: 'absolute',
            right: 0,
            bottom: 0,
            left: 0,
            height: 320,
            background: 'var(--aw-bg-nav)',
            borderTop: '1px solid var(--aw-border-control)',
          }}
        />
        <Story />
      </div>
    ),
  ],
}

export const PhoneShowsClosedSessionAndMergeFailure: Story = {
  name: 'The phone keeps a closed session and failed merge visible together.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    toastBottomOffset: null,
    actions: [
      {
        id: 1,
        kind: 'close',
        sessionId: 'example-session',
        sessionTitle: 'Review the example configuration',
        status: 'closed',
        createdAt: 1_800_000_000_000,
      },
      {
        id: 2,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example deployment',
        pullRequestNumber: 43,
        status: 'mergeFailed',
        error: 'The example branch changed after confirmation.',
        createdAt: 1_800_000_000_001,
      },
    ],
    toastIds: [2, 1],
  },
}
