import type { Meta, StoryObj } from '@storybook/react-vite'

import { MobileConversationActionsMenu } from '#components/mobile-conversation-actions-menu'

const meta = {
  component: MobileConversationActionsMenu,
  args: {
    sessionId: 'sample-session',
    isDone: false,
    isStarred: false,
    open: false,
    onLater: () => undefined,
    onToggleStarred: () => undefined,
    onEditTitle: () => undefined,
  },
} satisfies Meta<typeof MobileConversationActionsMenu>

export default meta
type Story = StoryObj<typeof meta>

export const Closed: Story = {
  name: 'The session actions menu stays closed until requested',
}

export const OpenForAStarredDoneSession: Story = {
  name: 'The open menu offers actions for a starred Done session',
  args: {
    isDone: true,
    isStarred: true,
    open: true,
    onClose: () => undefined,
  },
}
