import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

const session: Session = {
  id: 'example-session',
  category: 'needs_action',
  engine: 'claude',
  workType: 'development',
  isPaused: false,
  kind: 'Question',
  title: 'Update retry handling',
  preview: 'Should retries stop after three attempts?',
  elapsedSeconds: 8,
  unread: true,
  task: {
    id: 'example-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
    isClosed: false,
  },
  tasks: [
    {
      id: 'example-task',
      number: 2048,
      title: 'Improve request reliability',
      priority: 1,
      isClosed: false,
    },
    {
      id: 'example-doc-task',
      number: 2049,
      title: 'Document request retries',
      priority: null,
      isClosed: false,
    },
  ],
  repo: 'example/service',
  project: 'sample project',
  ancestorSessionIds: [],
  parent: null,
  pullRequest: null,
}

const meta = {
  component: SessionRow,
  args: {
    session,
  },
  decorators: [
    (Story) => (
      <div className="story-session-list">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionRow>

export default meta
type Story = StoryObj<typeof meta>

export const NeedsActionIsUnread: Story = {
  name: 'An unread question waits for a response',
}

export const FullInboxRow: Story = {
  name: 'A full row shows the request preview with session details',
  args: {
    density: 'full',
  },
}

export const CritReviewWaitShowsItsReason: Story = {
  name: 'A Crit review wait shows what needs review',
  args: {
    density: 'full',
    session: {
      ...session,
      kind: 'Review',
      isWaitingForCritReview: true,
      waitingReason: 'Review the retry policy changes',
    },
  },
}

export const CritReviewWaitWithoutReasonKeepsItsPreview: Story = {
  name: 'A Crit review wait without a reason keeps the existing preview',
  args: {
    density: 'full',
    session: {
      ...session,
      kind: 'Review',
      isWaitingForCritReview: true,
      waitingReason: null,
    },
  },
}

export const CritReviewWaitKeepsPendingTodoPreview: Story = {
  name: 'A pending todo keeps its preview during a Crit review wait',
  args: {
    density: 'full',
    session: {
      ...session,
      kind: 'Todo',
      isWaitingForCritReview: true,
      waitingReason: 'Review the retry policy changes',
    },
  },
}

export const CompactSessionRow: Story = {
  name: 'A compact row keeps space around its status and details',
  args: { density: 'compact' },
}

export const CompactPhoneSessionRow: Story = {
  name: 'A phone compact row keeps the task and repository on separate lines',
  args: { density: 'compact-phone' },
}

export const VibeEnabledSession: Story = {
  name: 'The enabled session shows a filled bolt beside its title',
  args: {
    session: { ...session, vibeMode: { mode: 'enabled' } },
  },
}

export const VibeInheritedSession: Story = {
  name: 'The inherited session shows an outlined bolt beside its title',
  args: {
    session: {
      ...session,
      vibeMode: { mode: 'inherited', ancestorSessionId: 'sample-parent' },
    },
  },
}

export const CompactSessionWithoutRole: Story = {
  name: 'A compact row keeps its role icon slot when no role is assigned',
  args: {
    density: 'compact',
    session: { ...session, workType: null },
  },
}

export const NeedsActionShowsApproval: Story = {
  name: 'An approval request waits for a response',
  args: {
    session: {
      ...session,
      kind: 'Approve',
      title: 'Allow the shell command',
      preview: 'This command needs your approval.',
    },
  },
}

export const NeedsActionShowsAReview: Story = {
  name: 'A review request waits for a response',
  args: {
    session: {
      ...session,
      kind: 'Review',
      title: 'Review the generated changes',
      preview: 'The draft is ready for review.',
    },
  },
}

export const NeedsActionShowsAPlan: Story = {
  name: 'A plan request waits for a response',
  args: {
    session: {
      ...session,
      kind: 'Plan',
      title: 'Review the implementation plan',
      preview: 'The agent is waiting for plan approval.',
    },
  },
}

export const NeedsActionShowsATodo: Story = {
  name: 'A todo request uses an empty square icon',
  args: {
    session: {
      ...session,
      kind: 'Todo',
      title: 'Update the onboarding guide',
      preview: 'Publish the revised guide.',
    },
  },
}

export const NeedsActionShowsAProposal: Story = {
  name: 'A proposal request uses a signpost icon',
  args: {
    session: {
      ...session,
      kind: 'Proposal',
      title: 'Choose a dashboard layout',
      preview: 'Select a navigation style.',
    },
  },
}

export const NeedsActionShowsAnUnmappedKind: Story = {
  name: 'An unmapped kind uses a generic question mark icon',
  args: {
    session: {
      ...session,
      kind: 'UnmappedKind',
      title: 'Review an experimental workflow',
      preview: 'The session uses an unrecognized kind.',
    },
  },
}

export const NeedsActionShowsAnError: Story = {
  name: 'An error request uses the red status color',
  args: {
    session: {
      ...session,
      kind: 'Error',
      title: 'Retry limit was reached',
      preview: 'The request failed after the final retry.',
    },
  },
}

export const NeedsActionShowsAStoppedSession: Story = {
  name: 'A stopped session uses the muted status color',
  args: {
    session: {
      ...session,
      kind: 'Stopped',
      title: 'Session stopped unexpectedly',
      unread: false,
    },
  },
}

export const WorkIsInProgress: Story = {
  name: 'An active session shows its working time',
  args: {
    session: {
      ...session,
      category: 'in_progress',
      kind: null,
      unread: false,
      elapsedSeconds: 30,
    },
  },
}

export const WorkIsWaitingForExternalWork: Story = {
  name: 'A Board compact waiting row shows why it is paused',
  args: {
    density: 'compact',
    showWaitingReason: true,
    session: {
      ...session,
      category: 'waiting',
      kind: null,
      waitingReason: 'Waiting for the checks to finish',
      unread: false,
    },
  },
}

export const WorkIsDone: Story = {
  name: 'A completed session shows its merged pull request',
  args: {
    session: {
      ...session,
      category: 'done',
      kind: null,
      unread: false,
      pullRequest: { number: 2049, state: 'Merged' },
      elapsedSeconds: 3600,
    },
  },
}

export const WorkIsDoneWithDecisionsToReview: Story = {
  name: 'A completed session shows decisions that still need review',
  args: {
    session: {
      ...session,
      category: 'done',
      kind: null,
      unread: false,
      decisionCount: 3,
      unreviewedDecisionCount: 2,
    },
  },
}

export const WorkIsDoneWithOneDecisionToReview: Story = {
  name: 'A completed session shows one decision that needs review',
  args: {
    session: {
      ...session,
      category: 'done',
      kind: null,
      unread: false,
      decisionCount: 1,
      unreviewedDecisionCount: 1,
    },
  },
}

export const WorkShowsDecisionCount: Story = {
  name: 'An active session shows its decision count',
  args: {
    session: {
      ...session,
      category: 'in_progress',
      kind: null,
      unread: false,
      decisionCount: 1,
      unreviewedDecisionCount: 1,
    },
  },
}

export const MobileInboxRow: Story = {
  name: 'A mobile Inbox row uses the full layout at phone size',
  args: {
    density: 'full',
    isMobile: true,
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{ width: '390px', minHeight: '180px' }}
      >
        <Story />
      </div>
    ),
  ],
}
