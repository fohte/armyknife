import type { Meta, StoryObj } from '@storybook/react-vite'

import { PermissionCard } from '#components/permission-card'
import type { ChatItem, SessionActions } from '#types'

const permission: ChatItem = {
  type: 'permission',
  requestId: 'permission-request-example',
  tool: 'Bash',
  description: '',
  command: `set -euo pipefail
printf '%s\\n' 'Preparing preview'
git add ./sample-config.yml && git commit -m "$(cat <<'EOF'
chore: refresh sample config

intent(sample): keep local settings in sync
EOF
)"`,
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: true, reject: true },
  interrupt: false,
  openInTerminal: true,
  rewind: false,
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

export const FileReadShowsDescription: Story = {
  name: 'A file-read request shows its description',
  args: {
    message: {
      type: 'permission',
      requestId: 'permission-request-read-example',
      tool: 'Read',
      description: 'The assistant wants to inspect a sample file.',
    },
  },
}

export const ApprovalIsSending: Story = {
  name: 'The approval button shows that the request is being sent',
  args: {
    pendingAction: {
      action: 'approve',
      status: 'sending',
      requestId: 'permission-request-example',
    },
  },
}

export const RejectionIsSending: Story = {
  name: 'The rejection button shows that the request is being sent',
  args: {
    pendingAction: {
      action: 'reject',
      status: 'sending',
      requestId: 'permission-request-example',
    },
  },
}

export const ApprovalWasSent: Story = {
  name: 'The card confirms that approval was sent',
  args: {
    pendingAction: {
      action: 'approve',
      status: 'sent',
      requestId: 'permission-request-example',
    },
  },
}

export const RejectionWasSent: Story = {
  name: 'The card confirms that rejection was sent',
  args: {
    pendingAction: {
      action: 'reject',
      status: 'sent',
      requestId: 'permission-request-example',
    },
  },
}
