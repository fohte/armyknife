import type { Meta, StoryObj } from '@storybook/react-vite'

import { CodeBlockCopyButton } from '#components/code-block-copy-button'

const meta = {
  component: CodeBlockCopyButton,
  args: {
    status: 'idle',
    onCopy: () => undefined,
  },
} satisfies Meta<typeof CodeBlockCopyButton>

export default meta
type Story = StoryObj<typeof meta>

export const ReadyToCopy: Story = {
  name: 'The button is ready to copy the code',
}

export const CopySucceeded: Story = {
  name: 'The button confirms that the code was copied',
  args: {
    status: 'copied',
  },
}

export const CopyFailed: Story = {
  name: 'The button announces when the code could not be copied',
  args: {
    status: 'failed',
  },
}
