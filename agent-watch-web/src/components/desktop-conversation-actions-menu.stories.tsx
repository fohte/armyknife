import type { Meta, StoryObj } from '@storybook/react-vite'

import { DesktopConversationActionsMenu } from '#components/desktop-conversation-actions-menu'

const meta = {
  component: DesktopConversationActionsMenu,
  args: { onClose: () => undefined },
} satisfies Meta<typeof DesktopConversationActionsMenu>

export default meta
type Story = StoryObj<typeof meta>

export const Closed: Story = {
  name: 'The header menu stays closed until requested',
}

export const Open: Story = {
  name: 'The open header menu offers a guarded close action',
  args: { open: true },
}
