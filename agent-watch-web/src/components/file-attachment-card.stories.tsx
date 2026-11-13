import type { Meta, StoryObj } from '@storybook/react-vite'

import { FileAttachmentCard } from '#components/file-attachment-card'

const meta = {
  component: FileAttachmentCard,
  args: {
    message: {
      type: 'file_attachment',
      files: [
        {
          name: 'sample-dashboard.png',
          source: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
          isImage: true,
          available: true,
        },
      ],
      caption: 'A sample dashboard for review.',
      timestamp: '2030-01-01T12:00:00Z',
    },
  },
} satisfies Meta<typeof FileAttachmentCard>

export default meta
type Story = StoryObj<typeof meta>

export const InlineImage: Story = {
  name: 'The card displays an image inline with its caption',
}

export const ExpandedImage: Story = {
  name: 'The image preview is expanded over the conversation',
  args: { previewOpen: true },
}

export const DownloadableFile: Story = {
  name: 'The card links a document for download',
  args: {
    message: {
      type: 'file_attachment',
      files: [
        {
          name: 'sample-report.pdf',
          source: '/api/sessions/fixture-session/files/fixture-file',
          isImage: false,
          available: true,
        },
      ],
      caption: null,
    },
  },
}

export const MissingFile: Story = {
  name: 'The card names a file that is no longer available',
  args: {
    message: {
      type: 'file_attachment',
      files: [
        {
          name: 'sample-report.pdf',
          source: '/api/sessions/fixture-session/files/fixture-file',
          isImage: false,
          available: false,
        },
      ],
      caption: 'The file could not be loaded.',
    },
  },
}
