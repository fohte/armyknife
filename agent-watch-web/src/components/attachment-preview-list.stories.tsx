import type { Meta, StoryObj } from '@storybook/react-vite'

import { AttachmentPreviewList } from '#components/attachment-preview-list'

const meta = {
  component: AttachmentPreviewList,
  args: {
    attachments: [
      {
        id: 'sample-image',
        name: 'screen.png',
        size: 42_000,
        previewUrl:
          'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
      },
      {
        id: 'sample-file',
        name: 'notes.txt',
        size: 2048,
        previewUrl: null,
      },
    ],
    onRemove: () => undefined,
  },
} satisfies Meta<typeof AttachmentPreviewList>

export default meta
type Story = StoryObj<typeof meta>

export const SelectedAttachments: Story = {
  name: 'Selected attachments show image previews and file sizes',
}

export const SubmittedAttachments: Story = {
  name: 'Submitted attachments stay visible without remove controls',
  args: { showRemove: false },
}
