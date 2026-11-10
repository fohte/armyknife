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
  name: 'the long active tool target stays on one line',
  args: {
    activeTool: {
      tool: 'exec',
      target:
        "printf '%s\\n' 'sample output' | rg --line-number 'sample' src/fixtures/long-named-example-file.rs && cargo test --package sample-package --test integration -- --exact long_command_is_kept_for_display",
    },
  },
}
