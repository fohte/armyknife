import type { Meta, StoryObj } from '@storybook/react-vite'

import { BangCommandDialog } from '#components/bang-command-dialog'

const meta = {
  title: 'Conversation/Bang command dialog',
  component: BangCommandDialog,
  args: {
    commands: ['git status --short'],
    cwd: '/workspace/example',
    onCancel: () => undefined,
    onConfirm: () => undefined,
    open: true,
  },
} satisfies Meta<typeof BangCommandDialog>

export default meta
type Story = StoryObj<typeof meta>

export const ConfirmsCommandAndDirectory: Story = {
  name: 'The confirmation shows the command and working directory',
}

export const ConfirmsEveryCommand: Story = {
  name: 'The confirmation lists every command before running them',
  args: { commands: ['git status --short', 'git diff --stat'] },
}

export const ShowsTheRunningState: Story = {
  name: 'The confirmation stays open while commands run',
  args: { running: true },
}

export const Closed: Story = {
  name: 'The dialog is closed before confirmation',
  args: { open: false },
}
