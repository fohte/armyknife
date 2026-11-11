import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionHeading } from '#components/session-heading'

const meta = {
  component: SessionHeading,
  args: {
    title: 'Investigate a failing build',
    isPaused: true,
    sessionId: 'sample-session-7f3a',
  },
} satisfies Meta<typeof SessionHeading>

export default meta
type Story = StoryObj<typeof meta>

export const PausedSession: Story = {
  name: 'A paused session keeps its title beside the copy button',
}

export const ActiveSession: Story = {
  args: { isPaused: false },
  name: 'An active session keeps its title beside the copy button',
}
