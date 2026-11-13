import type { Meta, StoryObj } from '@storybook/react-vite'

import { BangCommandButton } from '#components/bang-command-button'

const meta = {
  title: 'Conversation/Bang command button',
  component: BangCommandButton,
  args: {
    label: 'Run command',
    status: 'idle',
    onClick: () => undefined,
  },
} satisfies Meta<typeof BangCommandButton>

export default meta
type Story = StoryObj<typeof meta>

export const ReadyToRun: Story = {
  name: 'The command is ready to run',
}

export const Running: Story = {
  name: 'The command is running',
  args: { status: 'running' },
}

export const Succeeded: Story = {
  name: 'The command completed successfully',
  args: { label: 'Succeeded', status: 'succeeded', message: 'Completed' },
}

export const Failed: Story = {
  name: 'The command finished with an error',
  args: {
    label: 'Failed',
    status: 'failed',
    message: 'Failed (exit code 1)',
  },
}
