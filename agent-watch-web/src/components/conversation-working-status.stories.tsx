import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationWorkingStatus } from '#components/conversation-working-status'

const meta = {
  component: ConversationWorkingStatus,
  args: { elapsedSeconds: 92, activeTool: undefined },
  decorators: [
    (Story) => (
      <div style={{ width: '760px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConversationWorkingStatus>

export default meta
type Story = StoryObj<typeof meta>

export const WorkingWithoutAnActiveTool: Story = {
  name: 'The session is working without a known active tool',
}

export const WorkingWithAnActiveTool: Story = {
  name: 'The active tool and target appear beside the elapsed time',
  args: {
    activeTool: {
      tool: 'exec',
      target: 'cargo test --manifest-path fixture/Cargo.toml',
    },
  },
}
