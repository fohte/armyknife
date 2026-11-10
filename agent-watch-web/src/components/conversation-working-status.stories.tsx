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

export const CompactingConversation: Story = {
  name: 'The conversation is being compacted',
  args: { isCompacting: true },
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

export const WorkingWithBackgroundTasks: Story = {
  name: 'The working status lists tasks that are still running',
  args: {
    backgroundTasks: [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
      },
      {
        id: 'fixture-task-two',
        description: 'Review the generated report',
        elapsedSeconds: 43,
      },
    ],
  },
}

export const BackgroundTasksContinueWhileTheSessionWaits: Story = {
  name: 'Background tasks stay visible while the session waits',
  args: {
    showWorking: false,
    backgroundTasks: [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
      },
    ],
  },
}
