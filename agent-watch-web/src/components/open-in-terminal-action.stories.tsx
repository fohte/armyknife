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

export const Ready: Story = {
  name: 'The terminal action is ready to use',
  args: { status: null },
}

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

export const LongFailure: Story = {
  name: 'A long terminal error wraps inside the available width',
  args: {
    status: {
      state: 'error',
      message:
        'The terminal session could not be focused because its client is no longer available. Reopen the session in a terminal and try again.',
    },
  },
}
