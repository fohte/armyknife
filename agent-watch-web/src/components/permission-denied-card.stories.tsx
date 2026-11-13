import type { Meta, StoryObj } from '@storybook/react-vite'

import { PermissionDeniedCard } from '#components/permission-denied-card'
import type { ChatItem } from '#types'

const deniedOperation: ChatItem = {
  type: 'permission_denied',
  seq: 1,
  toolUseId: 'tool-call-example',
  tool: 'Bash',
  inputSummary: 'A shell command was denied.',
  command: `printf '%s\\n' 'Checking preview'
git status --short`,
  reason: '[Sensitive Operation]',
  retryable: true,
  resolved: false,
}

const meta = {
  component: PermissionDeniedCard,
  args: {
    message: deniedOperation,
    canRetry: true,
    actionsReady: true,
    pendingAction: null,
    onRetry: () => undefined,
  },
} satisfies Meta<typeof PermissionDeniedCard>

export default meta
type Story = StoryObj<typeof meta>

export const RetryAvailable: Story = {
  name: 'A classifier denial offers a one-time retry',
}

export const RetryIsSending: Story = {
  name: 'The retry action shows while its message is sending',
  args: {
    pendingAction: {
      action: 'retry',
      status: 'sending',
      requestId: 'permission-denied-1',
    },
  },
}

export const RetryWasSent: Story = {
  name: 'The card confirms that the retry instruction was sent',
  args: {
    message: { ...deniedOperation, resolved: true },
    pendingAction: {
      action: 'retry',
      status: 'sent',
      requestId: 'permission-denied-1',
    },
  },
}

export const NoClassifierVerdict: Story = {
  name: 'A denial without a classifier verdict has no retry action',
  args: {
    message: {
      ...deniedOperation,
      reason:
        'Auto mode could not evaluate this action and is blocking it for safety.',
      retryable: false,
    },
  },
}
