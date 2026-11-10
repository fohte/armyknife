import type { Meta, StoryObj } from '@storybook/react-vite'

import { PermissionCard } from '#components/permission-card'
import type { ChatItem, SessionActions } from '#types'

const permission: Extract<ChatItem, { type: 'permission' }> = {
  type: 'permission',
  tool: 'Shell',
  description: 'Run the formatter on changed files.',
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
}

const meta = {
  component: PermissionCard,
  args: {
    message: permission,
    actions,
    actionsReady: true,
    engine: 'Claude',
    onAction: () => Promise.resolve(true),
  },
} satisfies Meta<typeof PermissionCard>

export default meta
type Story = StoryObj<typeof meta>

export const PermissionWaitsForApproval: Story = {
  name: 'The permission request offers approval and rejection',
}

export const CodexPermissionNeedsTerminalReview: Story = {
  name: 'The Codex permission request directs review to the terminal',
  args: {
    actions: {
      message: true,
      answerQuestion: false,
      interrupt: true,
      openInTerminal: true,
    },
    engine: 'Codex',
  },
}
