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

export const TheWorkingStatusHasNoInlineDetail: Story = {
  name: 'The working row keeps its chevron at the right edge without an active tool',
  args: { state: state('working', null) },
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

export const OneDraftIsWaitingForReview: Story = {
  name: 'One issue comment draft is waiting for review',
  args: {
    state: state(
      'draft-review',
      '1 draft · Issue comment #1234 is in the editor',
    ),
  },
}

export const MultipleDraftsAreWaitingForReview: Story = {
  name: 'Four drafts are waiting while an issue comment is in the editor',
  args: {
    state: state(
      'draft-review',
      '4 drafts · Issue comment #1234 is in the editor',
    ),
  },
}

export const CritAndDraftReviewsAreWaiting: Story = {
  name: 'Crit and draft reviews are both waiting',
  args: {
    state: state(
      'draft-review',
      '4 drafts · Issue comment #1234 is in the editor · crit · PR #742',
    ),
  },
}

export const OneDraftIsWaitingForReviewOnMobile: Story = {
  name: 'The mobile band offers one issue comment draft for review',
  args: {
    state: state(
      'draft-review',
      '1 draft · Issue comment #1234 is in the editor',
    ),
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

export const MultipleDraftsAreWaitingForReviewOnMobile: Story = {
  name: 'The mobile band shows the count of four drafts',
  args: {
    state: state(
      'draft-review',
      '4 drafts · Issue comment #1234 is in the editor',
    ),
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

export const CritAndDraftReviewsAreWaitingOnMobile: Story = {
  name: 'The mobile band shows Crit and draft review waiting together',
  args: {
    state: state(
      'draft-review',
      '4 drafts · Issue comment #1234 is in the editor · crit · PR #742',
    ),
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

export const AllCritBackgroundTasksAreExpanded: Story = {
  name: 'The Crit review shows every background task',
  args: {
    state: state('crit-review', 'crit · PR #318'),
    backgroundTasks,
    defaultBackgroundTasksExpanded: true,
  },
}

export const AQuestionNeedsAnAnswer: Story = {
  name: 'The question row points back to its conversation card',
  args: { state: state('question', 'Which retry interval should be used?') },
}

export const AQuestionShowsBackgroundTasks: Story = {
  name: 'The question row shows its background tasks on mobile',
  args: {
    state: state('question', 'Which retry interval should be used?'),
    backgroundTasks,
    defaultBackgroundTasksExpanded: true,
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

export const APermissionNeedsApproval: Story = {
  name: 'The permission row points back to its conversation card',
  args: { state: state('permission', 'Bash  cargo run --package fictional') },
}

export const APermissionShowsBackgroundTasks: Story = {
  name: 'The permission row shows its background tasks',
  args: {
    state: state('permission', 'Bash  cargo run --package fictional'),
    backgroundTasks,
    defaultBackgroundTasksExpanded: true,
  },
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
