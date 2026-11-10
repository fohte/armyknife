import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationPaneView } from '#components/conversation-pane-view'
import type { ChatItem, Session, SessionActions } from '#types'

const session: Session = {
  id: 'sample-session',
  engine: 'Claude',
  category: 'needs_action',
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
        question: 'Which retry policy should be used?',
        choices: [
          'Retry three times with exponential backoff',
          'Retry once without delay',
        ],
      },
    ],
  },
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

export const CodexOnlyOpensInTerminal: Story = {
  name: 'A Codex session sends questions and permissions to the terminal',
  args: {
    session: { ...session, engine: 'Codex' },
    messages: [
      {
        type: 'question',
        question: 'Which retry policy should be used?',
        choices: ['Retry three times', 'Retry once'],
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
