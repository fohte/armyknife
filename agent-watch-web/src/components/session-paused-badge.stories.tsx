import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionPausedBadge } from '#components/session-paused-badge'

const meta = {
  component: SessionPausedBadge,
  args: { isPaused: true },
} satisfies Meta<typeof SessionPausedBadge>

export default meta
type Story = StoryObj<typeof meta>

export const SessionIsPaused: Story = {
  name: 'A paused session is labeled as paused',
}
