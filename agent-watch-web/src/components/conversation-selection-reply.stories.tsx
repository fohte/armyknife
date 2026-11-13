import type { Meta, StoryObj } from '@storybook/react-vite'

import { ConversationSelectionReply } from '#components/conversation-selection-reply'

const meta = {
  component: ConversationSelectionReply,
  args: {
    position: { left: 220, top: 180 },
    onReply: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ width: '100%', height: 360, padding: 32 }}>
        <p style={{ maxWidth: 440, marginTop: 120 }}>
          An agent response with a selected sentence ready to quote.
        </p>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ConversationSelectionReply>

export default meta
type Story = StoryObj<typeof meta>

export const SelectedTextCanBeRepliedTo: Story = {
  name: 'A reply menu floats beside selected text',
}
