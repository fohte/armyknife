import type { Meta, StoryObj } from '@storybook/react-vite'

import { PermissionCard } from '#components/permission-card'
import type { ChatItem, SessionActions } from '#types'

const permission: ChatItem = {
  type: 'permission',
  tool: 'Shell',
  description: 'Run the formatter on changed files.',
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: true, reject: true },
  interrupt: false,
  openInTerminal: true,
}

const callbacks = {
  onAction: () => undefined,
  onOpenInTerminal: () => undefined,
}

const meta = {
  component: PermissionCard,
  args: {
    message: permission,
    actions,
    actionsReady: true,
    pendingAction: null,
    isCodex: false,
    ...callbacks,
  },
} satisfies Meta<typeof PermissionCard>

export default meta
type Story = StoryObj<typeof meta>

export const WaitingForDecision: Story = {
  name: 'A permission request offers approval and rejection',
}

export const ApprovalIsSending: Story = {
  name: 'The approval button shows that the request is being sent',
  args: {
    pendingAction: { action: 'approve', status: 'sending' },
  },
}

export const RejectionIsSending: Story = {
  name: 'The rejection button shows that the request is being sent',
  args: {
    pendingAction: { action: 'reject', status: 'sending' },
  },
}

export const ApprovalWasSent: Story = {
  name: 'The card confirms that approval was sent',
  args: {
    pendingAction: { action: 'approve', status: 'sent' },
  },
}

export const RejectionWasSent: Story = {
  name: 'The card confirms that rejection was sent',
  args: {
    pendingAction: { action: 'reject', status: 'sent' },
  },
}
