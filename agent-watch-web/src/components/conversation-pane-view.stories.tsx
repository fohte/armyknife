import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationPaneView } from '#components/conversation-pane-view'
import type { ChatItem, Session, SessionActions } from '#types'

const session: Session = {
  id: 'sample-session',
  engine: 'Claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Add request retry handling',
  preview: 'Which retry policy should be used?',
  elapsedSeconds: 18,
  unread: true,
  task: {
    id: 'sample-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
  },
  repo: 'example/service',
  project: 'service refresh',
  parent: { id: 'parent-session', title: 'Service reliability' },
  ancestorSessionIds: ['parent-session'],
  pullRequest: { number: 2049, state: 'Open' },
}

const actions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
}

const callbacks = {
  onAction: () => Promise.resolve(true),
  onSelectSession: () => undefined,
  onBack: () => undefined,
  onLater: () => undefined,
}

const meta = {
  component: ConversationPaneView,
  args: {
    session,
    messages: [] as ChatItem[],
    actions,
    actionsReady: true,
    loading: false,
    sending: false,
    error: null,
    ...callbacks,
  },
  decorators: [
    (Story) => (
      <div style={{ height: '800px', display: 'flex' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConversationPaneView>

export default meta
type Story = StoryObj<typeof meta>

export const QuestionWaitsForAnAnswer: Story = {
  name: 'A question offers numbered choices',
  args: {
    messages: [
      {
        type: 'assistant',
        text: 'The retry flow is ready. Which policy should I use?',
      },
      {
        type: 'question',
        header: 'Retry policy',
        question: 'Which **retry policy** should be used?',
        choices: [
          {
            label: 'Retry **three times** with exponential backoff',
            description: 'Wait briefly before each retry.',
          },
          { label: 'Retry once without delay' },
        ],
        multi_select: false,
        answer: null,
      },
    ],
  },
}

export const QueuedPromptAppearsAfterConversation: Story = {
  name: 'A queued prompt appears below the delivered conversation',
  args: {
    messages: [
      { type: 'user', text: 'Please summarize the current changes.' },
      {
        type: 'assistant',
        text: 'I am reviewing the current changes.',
      },
      {
        type: 'queued_user',
        content: [{ type: 'text', text: 'Also check the error handling.' }],
      },
    ],
  },
}

export const QuestionShowsSelectedAnswer: Story = {
  name: 'A question marks the choice selected by the user',
  args: {
    messages: [
      {
        type: 'question',
        question: 'Which **retry policy** should be used?',
        choices: [
          { label: 'Retry **three times** with exponential backoff' },
          { label: 'Retry once without delay' },
        ],
        multi_select: false,
        answer: 'Retry once without delay',
      },
    ],
  },
}

export const QuestionShowsFreeformAnswer: Story = {
  name: 'A question shows an answer outside its listed choices',
  args: {
    messages: [
      {
        type: 'question',
        question: 'Which **retry policy** should be used?',
        choices: [
          { label: 'Retry three times with exponential backoff' },
          { label: 'Retry once without delay' },
        ],
        multi_select: false,
        answer: 'Use a short delay before each retry',
      },
    ],
  },
}

export const PausedSessionIsVisible: Story = {
  name: 'The conversation header shows when its session is paused',
  args: {
    session: {
      ...session,
      category: 'done',
      kind: null,
      isPaused: true,
      unread: false,
    },
  },
}

export const PausedSessionIsVisibleOnIPhone: Story = {
  name: 'The iPhone conversation header shows when its session is paused',
  args: {
    isMobile: true,
    session: {
      ...session,
      category: 'done',
      kind: null,
      isPaused: true,
      unread: false,
    },
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{ width: '390px', height: '844px', display: 'flex' }}
      >
        <Story />
      </div>
    ),
  ],
}

export const PermissionWaitsForApproval: Story = {
  name: 'A permission request offers approval and rejection',
  args: {
    messages: [
      {
        type: 'assistant',
        text: 'I need to run the project formatter before continuing.',
      },
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const PermissionApprovalIsSending: Story = {
  name: 'The approval button shows that the request is being sent',
  args: {
    pendingPermissionAction: {
      action: 'approve',
      status: 'sending',
      tool: 'Shell',
      description: 'Run the formatter on changed files.',
    },
    messages: [
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const PermissionRejectionIsSending: Story = {
  name: 'The rejection button shows that the request is being sent',
  args: {
    pendingPermissionAction: {
      action: 'reject',
      status: 'sending',
      tool: 'Shell',
      description: 'Run the formatter on changed files.',
    },
    messages: [
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const PermissionApprovalWasSent: Story = {
  name: 'The permission card confirms that approval was sent',
  args: {
    pendingPermissionAction: {
      action: 'approve',
      status: 'sent',
      tool: 'Shell',
      description: 'Run the formatter on changed files.',
    },
    messages: [
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const StatusPanelToggleIsActive: Story = {
  name: 'The conversation header shows an active status panel toggle',
  args: { statusPanelOpen: true },
}

export const ReviewGuidanceFitsAnIPhone: Story = {
  name: 'A review directs iPhone users to their Mac',
  args: {
    isMobile: true,
    session: {
      ...session,
      category: 'needs_action',
      kind: 'Review',
      critUrl: 'http://127.0.0.1:5000/review/sample',
    },
    messages: [
      {
        type: 'assistant',
        text: 'The implementation is ready for review.',
      },
    ],
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        data-layout="mobile"
        style={{ width: '390px', height: '844px', display: 'flex' }}
      >
        <Story />
      </div>
    ),
  ],
}

export const CodexOnlyOpensInTerminal: Story = {
  name: 'A Codex session sends questions and permissions to the terminal',
  args: {
    session: { ...session, engine: 'Codex' },
    messages: [
      {
        type: 'question',
        question: 'Which retry policy should be used?',
        choices: [{ label: 'Retry three times' }, { label: 'Retry once' }],
        multi_select: false,
        answer: null,
      },
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the project formatter.',
      },
    ],
    actions: {
      message: true,
      answerQuestion: false,
      interrupt: true,
      openInTerminal: true,
    },
  },
}

export const AMessageIsSending: Story = {
  name: 'The composer shows that the message is sending',
  args: {
    sending: true,
  },
}

export const ImagesCanBeDropped: Story = {
  name: 'The conversation highlights image drops',
  args: {
    fileDropActive: true,
  },
}

export const TheConversationShowsOperationSummaries: Story = {
  name: 'The transcript shows messages and operation summaries',
  args: {
    messages: [
      {
        type: 'user',
        from: 'Service reliability',
        text: 'Add bounded retries to the request client.',
      },
      {
        type: 'assistant',
        text: 'I will keep retries limited and preserve the original error.',
      },
      {
        type: 'tool_summary',
        summary: '6 actions · 2 files edited · 12 tests passed',
      },
      {
        type: 'assistant',
        text: 'The retry limit now applies to transient failures only.',
      },
    ],
    actions: null,
  },
}

export const TheConversationShowsDecisionRows: Story = {
  name: 'The conversation shows a provisional decision in its transcript',
  args: {
    session: { ...session, decisionCount: 1, unreviewedDecisionCount: 1 },
    messages: [
      {
        type: 'assistant',
        text: 'I will use a fixed retry limit while continuing the work.',
      },
      {
        type: 'decision',
        seq: 1,
        title: 'Use a fixed retry limit',
        why: 'The current service has no reliable timing data.',
        considered: ['Retry using the latest duration'],
        easyToChange: 'One configuration value in the CI workflow.',
        review: null,
        createdAt: '2026-11-11T03:25:59Z',
      },
    ],
  },
}

export const DoneSessionNeedsDecisionReview: Story = {
  name: 'A completed session highlights decisions that need review',
  args: {
    session: {
      ...session,
      category: 'done',
      kind: null,
      decisionCount: 2,
      unreviewedDecisionCount: 1,
    },
    decisionsPanelOpen: true,
    selectedDecisionSeq: 1,
    messages: [
      {
        type: 'decision',
        seq: 1,
        title: 'Use a fixed retry limit',
        why: 'The current service has no reliable timing data.',
        considered: ['Retry using the latest duration'],
        easyToChange: 'One configuration value in the CI workflow.',
        review: null,
        createdAt: '2026-11-11T03:25:59Z',
      },
    ],
  },
}

export const PeerMessagesAndEventsHaveTheirOwnDirection: Story = {
  name: 'The conversation separates peer messages from one-line events',
  args: {
    sessions: [
      session,
      {
        ...session,
        id: 'peer-session',
        title: 'Build verification',
        task: null,
        parent: null,
        ancestorSessionIds: [],
        pullRequest: null,
      },
    ],
    messages: [
      {
        type: 'user',
        text: 'Please verify the fictional build output.',
      },
      {
        type: 'peer',
        direction: 'to',
        session_id: 'peer-session',
        content: [
          { type: 'text', text: 'Please check the example validation step.' },
        ],
      },
      {
        type: 'peer',
        direction: 'from',
        session_id: 'peer-session',
        content: [{ type: 'text', text: 'The example step passed.' }],
      },
      {
        type: 'event',
        kind: 'armyknife',
        text: 'The delegated check has finished.',
        failed: false,
      },
      {
        type: 'event',
        kind: 'system',
        text: 'A fictional task was blocked.',
        failed: true,
      },
    ],
  },
}

export const AnActionFailureIsVisible: Story = {
  name: 'A failed action shows the server error',
  args: {
    messages: [
      {
        type: 'permission',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
    error: 'session pane is idle or unavailable',
  },
}

export const BackgroundTaskContinuesWhileTheSessionWaits: Story = {
  name: 'A background task remains visible while the session waits',
  args: {
    backgroundTasks: [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
      },
    ],
  },
}
