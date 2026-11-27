import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationMessageRow } from '#components/conversation-message-row'
import type { ChatItem, SessionActions, SessionLinkTarget } from '#types'

const actions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
  rewind: false,
}

const sessions: SessionLinkTarget[] = []
const callbacks = {
  onAction: () => Promise.resolve(true),
  onOpenInTerminal: () => undefined,
  onSelectSession: () => undefined,
}

const meta = {
  component: ConversationMessageRow,
  args: {
    actions,
    actionsReady: true,
    isMobile: false,
    message: {
      type: 'assistant',
      text: 'The retry flow is ready for review.',
    } satisfies ChatItem,
    pendingAction: null,
    openInTerminalStatus: null,
    sessionEngine: 'claude',
    sessions,
    ...callbacks,
  },
} satisfies Meta<typeof ConversationMessageRow>

export default meta
type Story = StoryObj<typeof meta>

export const AssistantMessage: Story = {
  name: 'The assistant message shows formatted conversation text',
}

export const NarrationInConversation: Story = {
  name: 'Narration follows the conversation in lighter text',
  args: {
    message: {
      type: 'narration',
      text: 'I will inspect the fictional request before answering.',
    },
  },
}

export const PermissionIncludesItsThoughtSummary: Story = {
  name: 'A permission request includes its grouped thought summary',
  args: {
    message: {
      type: 'permission',
      requestId: 'permission-request-fictional',
      tool: 'Read',
      description: 'Read the fictional source file.',
      items: [
        { type: 'thinking', text: 'Review the fictional source request.' },
        {
          type: 'tool_use',
          tool: 'Read',
          target: 'src/fixture.rs',
          failed: false,
        },
      ],
    },
  },
}

export const AnswerableQuestion: Story = {
  name: 'A pending question offers answer choices',
  args: {
    message: {
      type: 'question',
      requestId: 'ask-request-story',
      questions: [
        {
          header: 'Cache policy',
          question: 'Which cache window should be used?',
          choices: [{ label: 'Window A' }, { label: 'Window B' }],
          multi_select: false,
          answer: null,
        },
      ],
    },
  },
}

export const PermissionIsSending: Story = {
  name: 'A permission card shows an approval in progress',
  args: {
    message: {
      type: 'permission',
      requestId: 'permission-request-example',
      tool: 'Shell',
      description: 'Run the project formatter.',
    },
    pendingAction: {
      action: 'approve',
      status: 'sending',
      requestId: 'permission-request-example',
    },
  },
}

export const VibeModeTurnedOn: Story = {
  name: 'Turning on vibe mode appears as a centered event',
  args: {
    message: {
      type: 'event',
      kind: 'armyknife',
      text: 'Vibe mode turned on',
      failed: false,
      timestamp: new Date(2026, 10, 21, 10, 20).toISOString(),
    },
    now: new Date(2026, 10, 21, 10, 30),
  },
}

export const VibeModeTurnedOff: Story = {
  name: 'Turning off vibe mode appears as a centered event',
  args: {
    message: {
      type: 'event',
      kind: 'armyknife',
      text: 'Vibe mode turned off',
      failed: false,
      timestamp: new Date(2026, 10, 21, 10, 14).toISOString(),
    },
    now: new Date(2026, 10, 21, 10, 30),
  },
}
