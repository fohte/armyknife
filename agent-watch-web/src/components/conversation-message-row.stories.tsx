import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationMessageRow } from '#components/conversation-message-row'
import type { ChatItem, SessionActions, SessionLinkTarget } from '#types'

const actions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
}

const sessions: SessionLinkTarget[] = []
const callbacks = {
  onAction: () => Promise.resolve(true),
  onOpenInTerminal: () => undefined,
  onSelectDecision: () => undefined,
  onSelectSession: () => undefined,
}

const meta = {
  component: ConversationMessageRow,
  args: {
    actions,
    actionsReady: true,
    index: 0,
    isMobile: false,
    selected: false,
    decisionPosition: undefined,
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

export const SelectedDecision: Story = {
  name: 'A selected decision shows its position in the review',
  args: {
    selected: true,
    decisionPosition: '1 of 1',
    message: {
      type: 'decision',
      seq: 1,
      title: 'Retry with exponential delay',
      why: 'A short delay avoids repeating temporary failures immediately.',
      considered: ['Retry without delay'],
      easyToChange: 'Adjust the retry delay.',
      review: null,
      createdAt: '2026-11-11T09:00:00Z',
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
