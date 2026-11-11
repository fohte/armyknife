import type { Meta, StoryObj } from '@storybook/react-vite'

import { OpenInTerminalAction } from '#components/open-in-terminal-action'

const meta = {
  component: OpenInTerminalAction,
  args: {
    status: null,
    onClick: () => undefined,
  },
} satisfies Meta<typeof OpenInTerminalAction>

export default meta
type Story = StoryObj<typeof meta>

export const Sending: Story = {
  name: 'The terminal switch is in progress',
  args: { status: { state: 'sending' } },
}

export const Switched: Story = {
  name: 'The terminal switch succeeded',
  args: { status: { state: 'success' } },
}

export const Failed: Story = {
  name: 'The terminal switch shows its error',
  args: { status: { state: 'error', message: 'The session is unavailable.' } },
}
