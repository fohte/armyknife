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
    selectedActionId: null,
    retry: () => undefined,
    dismissToast: () => undefined,
    clearFinished: () => undefined,
    openActivity: () => undefined,
    closeActivity: () => undefined,
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
