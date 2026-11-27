import type { Meta, StoryObj } from '@storybook/react-vite'

import { Composer } from '#components/composer'

const imagePreview =
  'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs='

const meta = {
  component: Composer,
  args: {
    children: <textarea aria-label="Message" placeholder="Write a message" />,
    onAddAttachments: () => undefined,
    sendState: 'dim',
  },
  decorators: [
    (Story) => (
      <div style={{ width: 'min(712px, 100%)', padding: 16 }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof Composer>

export default meta
type Story = StoryObj<typeof meta>

export const EmptyComposer: Story = {
  name: 'The empty composer waits for a message',
}

export const ComposerWithText: Story = {
  name: 'The composer enables sending when text is entered',
  args: {
    children: (
      <textarea
        aria-label="Message"
        value="Please summarize the latest changes."
        readOnly
      />
    ),
    sendState: 'on',
  },
}

export const ComposerWithThumbnails: Story = {
  name: 'The composer shows removable thumbnails above the text area',
  args: {
    attachments: [
      {
        id: 'image-one',
        name: 'diagram.png',
        size: 42_000,
        previewUrl: imagePreview,
      },
      {
        id: 'image-two',
        name: 'screen.png',
        size: 38_000,
        previewUrl: imagePreview,
      },
    ],
    children: (
      <textarea
        aria-label="Message"
        value="What do these diagrams show?"
        readOnly
      />
    ),
    onRemoveAttachment: () => undefined,
    sendState: 'on',
  },
}

export const DimSendButton: Story = {
  name: 'The send button stays dim while the session starts',
  args: {
    children: (
      <textarea
        aria-label="Message"
        value="Prepare the session before sending."
        readOnly
      />
    ),
    sendState: 'dim',
    statusText: 'Starting session…',
  },
}

export const SendButtonOff: Story = {
  name: 'The send button is off while directory selection is required',
  args: {
    sendState: 'off',
    statusText: 'Choose a directory to start a session.',
  },
}

export const IPhoneComposer: Story = {
  name: 'The iPhone composer uses larger controls and centered status text',
  args: {
    attachments: [
      {
        id: 'mobile-image',
        name: 'diagram.png',
        size: 42_000,
        previewUrl: imagePreview,
      },
    ],
    children: (
      <textarea
        aria-label="Message"
        value="Check the attached diagram."
        readOnly
      />
    ),
    isMobile: true,
    onRemoveAttachment: () => undefined,
    sendState: 'on',
    statusText: 'Ready to send',
  },
  decorators: [
    (Story) => (
      <div
        style={{
          width: 'min(390px, 100%)',
          padding: 12,
          background: '#0a0a0a',
        }}
      >
        <Story />
      </div>
    ),
  ],
}
