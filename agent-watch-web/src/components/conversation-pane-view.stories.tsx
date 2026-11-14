import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import { ConversationPaneView } from '#components/conversation-pane-view'
import { SessionStatusPanel } from '#components/session-status-panel'
import type { ChatItem, Session, SessionActions } from '#types'

const session: Session = {
  id: 'sample-session',
  engine: 'claude',
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
  rewind: false,
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

const iPhoneViewportDecorator: Decorator = (Story) => (
  <div
    className="story-mobile-layout"
    data-layout="mobile"
    style={{ width: 'min(390px, 100%)', height: '844px', display: 'flex' }}
  >
    <Story />
  </div>
)

export const QuestionWaitsForAnAnswer: Story = {
  name: 'A question offers selectable choices',
  args: {
    messages: [
      {
        type: 'assistant',
        text: 'The retry flow is ready. Which policy should I use?',
      },
      {
        type: 'question',
        requestId: 'ask-request-retry-policy',
        questions: [
          {
            header: 'Cache policy',
            question: 'Which **cache window** should be used?',
            choices: [
              {
                label: 'Use a **brief window**',
                description: 'Refresh cached data soon.',
              },
              { label: 'Use an extended window' },
            ],
            multi_select: false,
            answer: null,
          },
        ],
      },
    ],
  },
}

export const PendingActionsAppearAboveComposer: Story = {
  name: 'Pending cards sit above the composer while answered cards stay in the conversation',
  args: {
    messages: [
      {
        type: 'assistant',
        text: 'The configuration review is complete.',
      },
      {
        type: 'proposal',
        seq: 31,
        title: 'Use a staged rollout',
        summary: 'The selected plan is recorded in the original position.',
        options: [
          {
            label: 'Staged rollout',
            description: null,
            pros: [],
            cons: [],
            recommended: false,
          },
          {
            label: 'Immediate rollout',
            description: null,
            pros: [],
            cons: [],
            recommended: false,
          },
        ],
        answer: { choice: 0, comment: null },
        resolved: false,
        resolution: null,
        comment: null,
      },
      {
        type: 'question',
        requestId: 'request-retry-policy',
        questions: [
          {
            header: 'Retry policy',
            question: 'Which retry policy should be used?',
            choices: [{ label: 'Retry once' }, { label: 'Do not retry' }],
            multi_select: false,
            answer: 'Retry once',
          },
        ],
      },
      {
        type: 'assistant',
        text: 'I am preparing the final change.',
      },
      {
        type: 'proposal',
        seq: 32,
        title: 'Choose the alert destination',
        summary: 'Select where operational updates should appear.',
        options: [
          {
            label: 'Dashboard',
            description: null,
            pros: [],
            cons: [],
            recommended: true,
          },
          {
            label: 'Email',
            description: null,
            pros: [],
            cons: [],
            recommended: false,
          },
        ],
        answer: null,
        resolved: false,
        resolution: null,
        comment: null,
      },
      {
        type: 'question',
        requestId: 'request-notification-detail',
        questions: [
          {
            header: 'Notification detail',
            question: 'Should the alert include a short summary?',
            choices: [{ label: 'Yes' }, { label: 'No' }],
            multi_select: false,
            answer: null,
          },
        ],
      },
      {
        type: 'permission',
        requestId: 'permission-request-notification',
        tool: 'Write',
        description: 'Update the notification settings.',
      },
      {
        type: 'todo',
        seq: 33,
        title: 'Review the alert destination',
        detail: 'Confirm that the selected destination is available.',
        completed: false,
        resolution: null,
        comment: null,
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
        questions: [
          {
            question: 'Which **cache window** should be used?',
            choices: [
              { label: 'Use a brief window' },
              { label: 'Use an extended window' },
            ],
            multi_select: false,
            answer: 'Use an extended window',
          },
        ],
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
        questions: [
          {
            question: 'Which **cache window** should be used?',
            choices: [
              { label: 'Use a brief window' },
              { label: 'Use an extended window' },
            ],
            multi_select: false,
            answer: 'Refresh the cache after each update',
          },
        ],
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
  decorators: [iPhoneViewportDecorator],
}

export const StarredCanBeChangedFromTheIPhoneConversation: Story = {
  name: 'The iPhone conversation header shows the active Starred toggle',
  args: {
    isMobile: true,
    isToday: true,
  },
  decorators: [iPhoneViewportDecorator],
}

export const IPhoneConversationShowsTheStatusPanel: Story = {
  name: 'The iPhone conversation shows the full-screen status panel',
  args: {
    isMobile: true,
    statusPanelOpen: true,
    backgroundTasks: [
      {
        id: 'sample-background-task',
        description: 'Check the request retry flow',
        elapsedSeconds: 42,
        kind: 'subagent',
        agentType: 'Explorer',
      },
    ],
    mobileStatusPanel: (
      <SessionStatusPanel
        tabs={[
          {
            id: 'tree',
            label: 'Tree',
            count: 1,
            visible: true,
            content: <div className="session-tree-feedback">Session tree</div>,
          },
          {
            id: 'crit',
            label: 'crit',
            count: 1,
            content: <div className="session-tree-feedback">Crit review</div>,
          },
          {
            id: 'decisions',
            label: 'Decisions',
            count: 1,
            content: <div className="session-tree-feedback">Decisions</div>,
          },
          {
            id: 'running',
            label: 'Running',
            count: 1,
            content: (
              <div className="session-tree-feedback">Background tasks</div>
            ),
          },
        ]}
        activeTabId="tree"
        onTabChange={() => undefined}
        onClose={() => undefined}
      />
    ),
  },
  decorators: [iPhoneViewportDecorator],
}

export const IPhoneConversationShowsAQuestion: Story = {
  name: 'The iPhone conversation shows messages and a question card',
  args: {
    isMobile: true,
    actions: { ...actions, interrupt: false },
    messages: [
      {
        type: 'user',
        from: 'Example project lead',
        text: 'Please add retry handling to the request client.',
      },
      {
        type: 'assistant',
        text: 'I added bounded retries. Which policy should I use?',
      },
      {
        type: 'tool_summary',
        summary: 'Edit client.ts · Read ×3 · Shell ×1',
      },
      {
        type: 'decision',
        seq: 1,
        title: 'Retry with exponential delay',
        why: 'A short delay avoids repeating temporary failures immediately.',
        considered: ['Retry without a delay'],
        easyToChange: 'Update the retry delay constant.',
        review: null,
        createdAt: '2026-11-01T03:25:59Z',
      },
      {
        type: 'question',
        requestId: 'ask-request-mobile',
        questions: [
          {
            header: 'Cache policy',
            question: 'Should this cache use its default window?',
            choices: [
              { label: 'Yes, use the default window' },
              { label: 'No, choose another window' },
            ],
            multi_select: false,
            answer: null,
          },
        ],
      },
    ],
  },
  decorators: [iPhoneViewportDecorator],
}

export const IPhoneConversationShowsMultiplePendingCards: Story = {
  name: 'The iPhone composer keeps multiple pending cards within reach',
  args: {
    isMobile: true,
    actions: { ...actions, interrupt: false },
    messages: [
      {
        type: 'assistant',
        text: 'Choose a rollout plan and confirm the alert details.',
      },
      {
        type: 'proposal',
        seq: 41,
        title: 'Choose how to roll out the retry policy',
        summary:
          'The selected plan determines how quickly the updated retry behavior reaches users.',
        options: [
          {
            label: 'Gradual rollout',
            description: 'Increase traffic in measured steps.',
            pros: ['Limits the impact of an unexpected regression.'],
            cons: ['Takes longer to reach all users.'],
            recommended: true,
          },
          {
            label: 'Immediate rollout',
            description: 'Enable the policy for everyone at once.',
            pros: ['Makes the improvement available sooner.'],
            cons: ['A regression would affect more users.'],
            recommended: false,
          },
          {
            label: 'Preview group',
            description: 'Enable the policy for a small group first.',
            pros: ['Provides early feedback from real traffic.'],
            cons: ['Requires a separate follow-up rollout.'],
            recommended: false,
          },
        ],
        answer: null,
        resolved: false,
        resolution: null,
        comment: null,
      },
      {
        type: 'todo',
        seq: 42,
        title: 'Confirm the rollout window',
        detail: 'Check that the staged rollout fits the release schedule.',
        completed: false,
        resolution: null,
        comment: null,
      },
      {
        type: 'question',
        requestId: 'request-mobile-alert-details',
        questions: [
          {
            header: 'Alert destination',
            question: 'Where should rollout alerts be sent?',
            choices: [{ label: 'Dashboard' }, { label: 'Email' }],
            multi_select: false,
            answer: null,
          },
          {
            header: 'Alert detail',
            question: 'What detail should each alert include?',
            choices: [
              { label: 'A short summary' },
              { label: 'The full change details' },
            ],
            multi_select: false,
            answer: null,
          },
          {
            header: 'Alert frequency',
            question: 'How often should progress updates be sent?',
            choices: [{ label: 'At each stage' }, { label: 'At completion' }],
            multi_select: false,
            answer: null,
          },
        ],
      },
    ],
  },
  decorators: [iPhoneViewportDecorator],
}

export const IPhoneConversationShowsItsWorkingStatus: Story = {
  name: 'The iPhone conversation keeps messages and working status above the composer',
  args: {
    isMobile: true,
    session: { ...session, category: 'in_progress', kind: null },
    messages: [
      {
        type: 'user',
        text: 'Please verify the retry limits before the next release.',
      },
      {
        type: 'assistant',
        text: 'The request client now caps attempts and preserves the original failure.',
      },
      {
        type: 'tool_summary',
        summary: 'Read ×2 · Edit ×1 · Shell ×1',
      },
      {
        type: 'assistant',
        text: 'I am checking the timeout path and the final error message.',
      },
      {
        type: 'tool_in_progress',
        tool: 'Shell',
        target: 'cargo test --package example-client',
      },
    ],
  },
  decorators: [iPhoneViewportDecorator],
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
        requestId: 'permission-request-main',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const PermissionApprovalIsSending: Story = {
  name: 'The approval button shows that the request is being sent',
  args: {
    pendingPermissionActions: {
      'permission-request-main': {
        action: 'approve',
        status: 'sending',
        requestId: 'permission-request-main',
      },
    },
    messages: [
      {
        type: 'permission',
        requestId: 'permission-request-main',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const PermissionRejectionIsSending: Story = {
  name: 'The rejection button shows that the request is being sent',
  args: {
    pendingPermissionActions: {
      'permission-request-main': {
        action: 'reject',
        status: 'sending',
        requestId: 'permission-request-main',
      },
    },
    messages: [
      {
        type: 'permission',
        requestId: 'permission-request-main',
        tool: 'Shell',
        description: 'Run the formatter on changed files.',
      },
    ],
  },
}

export const PermissionApprovalWasSent: Story = {
  name: 'The permission card confirms that approval was sent',
  args: {
    pendingPermissionActions: {
      'permission-request-main': {
        action: 'approve',
        status: 'sent',
        requestId: 'permission-request-main',
      },
    },
    messages: [
      {
        type: 'permission',
        requestId: 'permission-request-main',
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

export const IPhoneReviewHeaderOffersStatusPanel: Story = {
  name: 'The iPhone review header offers the status panel toggle',
  args: {
    isMobile: true,
    statusPanelOpen: false,
    session: {
      ...session,
      category: 'needs_action',
      kind: 'Review',
      webUis: [
        {
          name: 'Design editor',
          url: 'http://127.0.0.1:5000/design/sample/',
        },
      ],
    },
    messages: [
      {
        type: 'assistant',
        text: 'The implementation is ready for review.',
      },
    ],
  },
  decorators: [iPhoneViewportDecorator],
}

export const CodexOnlyOpensInTerminal: Story = {
  name: 'A Codex session sends questions and permissions to the terminal',
  args: {
    session: { ...session, engine: 'codex' },
    messages: [
      {
        type: 'question',
        questions: [
          {
            question: 'Which cache window should be used?',
            choices: [{ label: 'Window A' }, { label: 'Window B' }],
            multi_select: false,
            answer: null,
          },
        ],
      },
      {
        type: 'permission',
        requestId: 'permission-request-review',
        tool: 'Shell',
        description: 'Run the project formatter.',
      },
    ],
    actions: {
      message: true,
      answerQuestion: false,
      interrupt: true,
      openInTerminal: true,
      rewind: false,
    },
  },
}

export const AMessageIsSending: Story = {
  name: 'The composer shows that the message is sending',
  args: {
    sending: true,
  },
}

export const InterruptRequestIsSending: Story = {
  name: 'The interrupt control shows progress while its request is sending',
  args: {
    interruptPending: true,
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

export const DesktopHeaderShowsClosedStatusPanelAndStarredControl: Story = {
  name: 'The desktop header shows closed status and Starred controls',
  args: {
    backgroundTasks: [
      {
        id: 'sample-background-task',
        description: 'Inspect the request flow',
        elapsedSeconds: 12,
        kind: 'subagent',
        agentType: 'Inspector',
      },
    ],
    isToday: false,
  },
}

export const DesktopHeaderShowsOpenStatusPanelAndStarredControl: Story = {
  name: 'The desktop header shows open status and an active Starred control',
  args: {
    backgroundTasks: [
      {
        id: 'sample-background-task',
        description: 'Inspect the request flow',
        elapsedSeconds: 12,
        kind: 'subagent',
        agentType: 'Inspector',
      },
    ],
    isToday: true,
    statusPanelOpen: true,
  },
}

export const DoneSessionCanBeClosed: Story = {
  name: 'A Done session exposes its Close action',
  args: {
    session: { ...session, category: 'done', kind: null },
    onClose: () => undefined,
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
        requestId: 'permission-request-error',
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
    isMobile: true,
    backgroundTasks: [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
        kind: 'background_task',
      },
    ],
  },
}

export const OlderMessagesCanBeLoaded: Story = {
  name: 'The conversation offers older messages above the loaded page',
  args: {
    start: 40,
    messages: [
      { type: 'user', text: 'Check the example request flow.' },
      {
        type: 'assistant',
        text: 'The earlier part of the conversation is available on demand.',
      },
    ],
  },
}

export const OlderMessagesAreLoading: Story = {
  name: 'The conversation shows progress while older messages load',
  args: {
    start: 40,
    loadingOlder: true,
    messages: [
      { type: 'user', text: 'Check the example request flow.' },
      {
        type: 'assistant',
        text: 'The earlier part of the conversation is available on demand.',
      },
    ],
  },
}

export const CompactBoundarySeparatesConversation: Story = {
  name: 'A divider marks where the conversation was compacted',
  args: {
    messages: [
      {
        type: 'assistant',
        text: 'The earlier part of the conversation is summarized.',
      },
      {
        type: 'event',
        kind: 'compact_boundary',
        text: 'Context compacted',
        failed: false,
      },
      {
        type: 'assistant',
        text: 'The assistant continues after the conversation is compacted.',
      },
    ],
  },
}
