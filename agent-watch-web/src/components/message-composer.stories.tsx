import type { Meta, StoryObj } from '@storybook/react-vite'

import { MessageComposer } from '#components/message-composer'

const meta = {
  component: MessageComposer,
  args: {
    draft: '',
    placeholder: 'Message session',
    disabled: false,
    isSending: false,
    isMobile: false,
    canInterrupt: false,
    onDraftChange: () => undefined,
    onKeyDown: () => undefined,
    onSend: () => undefined,
    onInterrupt: () => undefined,
  },
  decorators: [
    (Story) => (
      <div style={{ height: '800px', display: 'flex', alignItems: 'end' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof MessageComposer>

export default meta
type Story = StoryObj<typeof meta>

export const EmptyComposer: Story = {
  name: 'The composer is ready for a new message',
}

export const ComposerWithDraft: Story = {
  name: 'The composer expands to show a longer draft',
  args: {
    draft:
      'Add bounded retries to the request client. Keep the original error when all attempts fail, and make the delay grow between attempts.',
  },
}

export const MobileComposer: Story = {
  name: 'The composer shows touch sized controls on mobile',
  args: {
    isMobile: true,
    canInterrupt: true,
  },
  decorators: [
    (Story) => (
      <div
        style={{
          width: '390px',
          height: '844px',
          display: 'flex',
          alignItems: 'end',
          background: '#09090b',
        }}
      >
        <Story />
      </div>
    ),
  ],
}
