import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConfirmDialog } from '#components/confirm-dialog'

const meta = {
  component: ConfirmDialog,
  args: {
    title: 'Close without answering?',
    description:
      'It is waiting for your answer to a question. Closing ends the session, and the question stays unanswered.',
    confirmLabel: 'Close session',
    confirmVariant: 'danger',
    onCancel: () => undefined,
    onConfirm: () => undefined,
  },
} satisfies Meta<typeof ConfirmDialog>

export default meta
type Story = StoryObj<typeof meta>

export const DestructiveAction: Story = {
  name: 'The dialog highlights a destructive action',
}

export const MobileSheet: Story = {
  name: 'The mobile dialog appears as a bottom sheet',
  args: { mobile: true },
}

export const InlineVibeConfirmation: Story = {
  name: 'The inline dialog presents a message before sending',
  args: {
    title: 'Vibe off',
    description: 'Turning on vibe mode allows submission without review.',
    confirmLabel: 'Turn on and send',
    confirmVariant: 'default',
    presentation: 'inline',
    children: (
      <blockquote>
        watch-web からユーザーが vibe mode を on にしました。
      </blockquote>
    ),
  },
}
