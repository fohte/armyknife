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
          previewType: 'image',
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

export const DownloadablePdf: Story = {
  name: 'The card opens a PDF and keeps its download link',
  args: {
    message: {
      type: 'file_attachment',
      files: [
        {
          name: 'sample-report.pdf',
          source: '/api/sessions/fixture-session/files/fixture-file',
          isImage: false,
          previewType: 'pdf',
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
          previewType: 'pdf',
          available: false,
        },
      ],
      caption: 'The file could not be loaded.',
    },
  },
}

export const HtmlIsDownloadOnly: Story = {
  name: 'The card offers an HTML file as a download without previewing it',
  args: {
    message: {
      type: 'file_attachment',
      files: [
        {
          name: 'sample-page.html',
          source: '/api/sessions/fixture-session/files/fixture-html',
          isImage: false,
          previewType: 'html',
          available: true,
        },
      ],
      caption: null,
    },
  },
}
