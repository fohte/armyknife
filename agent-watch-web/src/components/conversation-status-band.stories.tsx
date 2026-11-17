import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationStatusBand } from '#components/conversation-status-band'
import type { ConversationStatusBandState } from '#components/conversation-status-band-state'

const meta = {
  component: ConversationStatusBand,
  args: {
    elapsedSeconds: 92,
    activeTool: undefined,
    backgroundTasks: [],
  },
  decorators: [
    (Story) => (
      <div style={{ width: '760px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConversationStatusBand>

export default meta
type Story = StoryObj<typeof meta>

const state = (
  kind: ConversationStatusBandState['kind'],
  detail: string | null,
): ConversationStatusBandState => ({ kind, detail })

export const TheSessionShowsItsActiveTool: Story = {
  name: 'The session shows an active tool and elapsed time',
  args: {
    state: state('working', null),
    activeTool: { tool: 'exec', target: 'cargo test --package sample' },
  },
}

export const TheConversationIsBeingSummarized: Story = {
  name: 'The conversation is being summarized',
  args: { state: state('compacting', null) },
}

export const BackgroundTasksAreRunning: Story = {
  name: 'The session is waiting for background tasks',
  args: {
    state: state('background-tasks', 'Check the sample build'),
    backgroundTasks: [
      {
        id: 'fixture-background-task-one',
        description: 'Check the sample build',
        elapsedSeconds: 128,
        kind: 'background_task',
      },
    ],
  },
}

export const CritReviewIsWaiting: Story = {
  name: 'The session is waiting for a Crit review',
  args: { state: state('crit-review', 'crit · PR #742') },
}

export const CritReviewButtonFillsTheMobileWidth: Story = {
  name: 'The mobile review button fills the available width',
  args: {
    state: state('crit-review', 'crit · PR #742'),
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '375px' }}>
        <Story />
      </div>
    ),
  ],
}

export const CritReviewAndBackgroundTasksAreWaiting: Story = {
  name: 'The Crit review is available while background tasks continue',
  args: {
    state: state('crit-review', 'crit · PR #742'),
    backgroundTasks: [
      {
        id: 'fixture-background-task-two',
        description: 'Check the sample build',
        elapsedSeconds: 128,
        kind: 'background_task',
      },
    ],
  },
}

export const AQuestionNeedsAnAnswer: Story = {
  name: 'The session is waiting for an answer to its question',
  args: { state: state('question', 'Which sample should be selected?') },
}

export const APermissionNeedsApproval: Story = {
  name: 'The session is waiting for permission approval',
  args: { state: state('permission', 'Bash  pnpm run validate') },
}

export const TheSessionIsWaitingForExternalWork: Story = {
  name: 'The session explains which external work it is waiting for',
  args: { state: state('waiting', 'Waiting for a sample check') },
}
