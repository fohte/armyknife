import type { Decorator, Meta, StoryObj } from '@storybook/react-vite'

import { MessageComposer } from '#components/message-composer'

const meta = {
  component: MessageComposer,
  args: {
    draft: '',
    placeholder: 'Message session',
    disabled: false,
    isSending: false,
    isProcessingAttachments: false,
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
const mobileViewportDecorator: Decorator = (Story) => (
  <div
    style={{
      width: 'min(390px, 100%)',
      height: '844px',
      display: 'flex',
      alignItems: 'end',
      background: '#09090b',
    }}
  >
    <Story />
  </div>
)

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

export const ComposerWithSkillSuggestions: Story = {
  name: 'The composer filters skills after the slash trigger',
  args: {
    draft: '/deb',
    engine: 'claude',
    skills: [
      { name: 'design-flow', description: 'Help shape a design.' },
      { name: 'debug-flow', description: 'Investigate and fix errors.' },
    ],
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
  name: 'The mobile composer places send and interrupt beside the text field',
  args: {
    isMobile: true,
    canInterrupt: true,
  },
  decorators: [mobileViewportDecorator],
}

export const MobileComposerWithoutInterrupt: Story = {
  name: 'The mobile composer sends without an interrupt control',
  args: {
    isMobile: true,
  },
  decorators: [mobileViewportDecorator],
}

export const MobileComposerWithAttachment: Story = {
  name: 'The mobile composer shows an attachment without an interrupt control',
  args: {
    isMobile: true,
    attachments: [
      {
        id: 'example-attachment',
        name: 'example-notes.txt',
        size: 1024,
        previewUrl: null,
      },
    ],
  },
  decorators: [mobileViewportDecorator],
}

export const MobileComposerWithAttachmentAndInterrupt: Story = {
  name: 'The mobile composer shows an attachment below the text and action controls',
  args: {
    isMobile: true,
    canInterrupt: true,
    attachments: [
      {
        id: 'example-attachment',
        name: 'example-notes.txt',
        size: 1024,
        previewUrl: null,
      },
    ],
  },
  decorators: [mobileViewportDecorator],
}
