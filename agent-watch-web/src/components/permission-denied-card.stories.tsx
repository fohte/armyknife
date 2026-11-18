import '#components/mobile-layout.css'

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
  resolution: null,
}

const meta = {
  component: PermissionDeniedCard,
  args: {
    message: deniedOperation,
    canRetry: true,
    actionsReady: true,
    pendingAction: null,
    canDismiss: true,
    onRetry: () => undefined,
    onDismiss: () => undefined,
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
    message: { ...deniedOperation, resolved: true, resolution: 'approved' },
    pendingAction: {
      action: 'retry',
      status: 'sent',
      requestId: 'permission-denied-1',
    },
  },
}

export const NoClassifierVerdict: Story = {
  name: 'A denial without a classifier verdict can still be dismissed',
  args: {
    message: {
      ...deniedOperation,
      reason:
        'Auto mode could not evaluate this action and is blocking it for safety.',
      retryable: false,
    },
  },
}

export const Dismissed: Story = {
  name: 'A dismissed denial stays collapsed at its original time',
  args: {
    message: { ...deniedOperation, resolved: true, resolution: 'dismissed' },
  },
}

export const DismissedExpanded: Story = {
  name: 'A dismissed denial shows its details when expanded',
  args: {
    message: { ...deniedOperation, resolved: true, resolution: 'dismissed' },
    defaultOpen: true,
  },
}

export const DismissIsSending: Story = {
  name: 'The dismiss action shows while the request is sending',
  args: {
    pendingAction: {
      action: 'dismiss',
      status: 'sending',
      requestId: 'permission-denied-1',
    },
  },
}

export const MobileActions: Story = {
  name: 'The retry and dismiss actions fit a mobile row',
  render: (args) => (
    <div
      className="conversation-pane"
      data-mobile="true"
      style={{ width: 390 }}
    >
      <PermissionDeniedCard {...args} />
    </div>
  ),
}
