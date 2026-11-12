import type { Meta, StoryObj } from '@storybook/react-vite'

import { StarredSessionToggle } from '#components/starred-session-toggle'

const meta = {
  component: StarredSessionToggle,
  args: {
    className: 'conversation-today-toggle',
    size: 'desktop',
    isStarred: false,
    onToggleToday: () => undefined,
  },
} satisfies Meta<typeof StarredSessionToggle>

export default meta
type Story = StoryObj<typeof meta>

export const DesktopSessionCanBeStarred: Story = {
  name: 'The desktop control offers to add the session to Starred',
}

export const DesktopSessionIsStarred: Story = {
  name: 'The desktop control shows that the session is in Starred',
  args: { isStarred: true },
}

export const MobileSessionIsStarred: Story = {
  name: 'The mobile control shows that the session is in Starred',
  args: {
    className: 'mobile-conversation-today',
    size: 'mobile',
    isStarred: true,
  },
}
