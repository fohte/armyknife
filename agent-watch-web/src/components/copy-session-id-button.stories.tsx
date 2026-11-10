import type { Meta, StoryObj } from '@storybook/react-vite'

import { CopySessionIdButton } from '#components/copy-session-id-button'

const meta = {
  component: CopySessionIdButton,
  args: {
    status: 'idle',
    compact: false,
    onCopy: () => undefined,
  },
} satisfies Meta<typeof CopySessionIdButton>

export default meta
type Story = StoryObj<typeof meta>

export const ReadyToCopy: Story = {
  name: 'The button is ready to copy the session ID',
}

export const CopySucceeded: Story = {
  name: 'The button confirms that the session ID was copied',
  args: {
    status: 'copied',
  },
}

export const CopyFailed: Story = {
  name: 'The button shows when the session ID could not be copied',
  args: {
    status: 'failed',
  },
}
