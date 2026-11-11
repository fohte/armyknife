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
    attachments: [],
    onDraftChange: () => undefined,
    onKeyDown: () => undefined,
    onPasteFiles: () => undefined,
    onRemoveAttachment: () => undefined,
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

export const ComposerWithImageAttachment: Story = {
  name: 'The composer shows a pasted image before sending',
  args: {
    attachments: [
      {
        id: 'image-1',
        name: 'screenshot.gif',
        size: 1024,
        previewUrl:
          'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
      },
    ],
  },
}

export const ComposerWithDocumentAttachment: Story = {
  name: 'The composer shows a document name and size before sending',
  args: {
    attachments: [
      {
        id: 'document-1',
        name: 'design-mock.html',
        size: 12 * 1024 * 1024,
        previewUrl: null,
      },
    ],
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
