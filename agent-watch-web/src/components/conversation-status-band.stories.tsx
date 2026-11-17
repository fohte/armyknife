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

const backgroundTasks = [
  {
    id: 'fixture-background-task-one',
    description: 'Rebuild the sample index',
    elapsedSeconds: 128,
    kind: 'background_task' as const,
  },
  {
    id: 'fixture-background-task-two',
    description: 'Check the sample bundle',
    elapsedSeconds: 62,
    kind: 'background_task' as const,
  },
]

export const TheSessionShowsItsActiveTool: Story = {
  name: 'The session shows its active tool in one line',
  args: {
    state: state('working', null),
    activeTool: { tool: 'exec', target: 'cargo test --package fictional' },
  },
}

export const TheMobileStatusRowUsesTheCompactHeight: Story = {
  name: 'The mobile status row fits within forty-four pixels',
  args: {
    state: state('waiting', 'Waiting for the sample check'),
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '390px' }}>
        <Story />
      </div>
    ),
  ],
}

export const TheActiveToolDetailsAreExpanded: Story = {
  name: 'The active tool details are expanded',
  args: {
    state: state('working', null),
    activeTool: { tool: 'exec', target: 'cargo test --package fictional' },
    defaultExpanded: true,
  },
}

export const TheConversationIsBeingSummarized: Story = {
  name: 'The conversation is being summarized in one line',
  args: { state: state('compacting', null) },
}

export const TheCompactingDetailsAreExpanded: Story = {
  name: 'The summary details are expanded',
  args: { state: state('compacting', null), defaultExpanded: true },
}

export const BackgroundTasksAreRunning: Story = {
  name: 'Background tasks are summarized in one line',
  args: {
    state: state('background-tasks', 'Rebuild the sample index'),
    backgroundTasks,
  },
}

export const AllBackgroundTaskDetailsAreExpanded: Story = {
  name: 'The expanded status shows every background task',
  args: {
    state: state('background-tasks', 'Rebuild the sample index'),
    backgroundTasks,
    defaultExpanded: true,
  },
}

export const CritReviewIsWaiting: Story = {
  name: 'The session is waiting for a Crit review',
  args: { state: state('crit-review', 'crit · PR #318') },
}

export const CritReviewButtonFillsTheMobileWidth: Story = {
  name: 'The mobile review button fills the available width',
  args: {
    state: state('crit-review', 'crit · PR #318'),
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '390px' }}>
        <Story />
      </div>
    ),
  ],
}

export const CritReviewAndBackgroundTasksAreWaiting: Story = {
  name: 'The Crit review stays available while background tasks run',
  args: {
    state: state('crit-review', 'crit · PR #318'),
    backgroundTasks,
  },
}

export const AQuestionNeedsAnAnswer: Story = {
  name: 'The question row points back to its conversation card',
  args: { state: state('question', 'Which retry interval should be used?') },
}

export const APermissionNeedsApproval: Story = {
  name: 'The permission row points back to its conversation card',
  args: { state: state('permission', 'Bash  cargo run --package fictional') },
}

export const TheSessionIsWaitingForExternalWork: Story = {
  name: 'The external work reason is summarized in one line',
  args: { state: state('waiting', 'Waiting for the sample check') },
}

export const TheWaitingReasonIsExpanded: Story = {
  name: 'The full external work reason is expanded',
  args: {
    state: state('waiting', 'Waiting for the sample check to finish'),
    defaultExpanded: true,
  },
}

export const IdleSessionsHaveNoBand: Story = {
  name: 'The idle session has no status band',
  args: { state: null },
}
