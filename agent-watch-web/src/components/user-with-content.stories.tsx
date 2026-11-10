import type { Meta, StoryObj } from '@storybook/react-vite'

import { UserWithContent } from '#components/user-with-content'

const meta = {
  component: UserWithContent,
  args: {
    message: {
      type: 'user_with_content',
      content: [
        { type: 'text', text: 'Please inspect this screenshot: ' },
        {
          type: 'image',
          source: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
          available: true,
        },
        { type: 'text', text: ' and retry the check.' },
      ],
      from: 'Example session',
    },
  },
} satisfies Meta<typeof UserWithContent>

export default meta
type Story = StoryObj<typeof meta>

export const ImageAppearsBetweenUserText: Story = {
  name: 'The user message shows an image between its text',
}

export const ExpiredImageIsExplained: Story = {
  name: 'The user message explains that its image is no longer available',
  args: {
    message: {
      type: 'user_with_content',
      content: [
        { type: 'text', text: 'Please inspect this screenshot.' },
        {
          type: 'image',
          source: '/api/images/00000000-0000-4000-8000-000000000001.png',
          available: false,
        },
      ],
    },
  },
}

export const TextOnlyMessageIsDisplayed: Story = {
  name: 'A text-only user message appears without image placeholders',
  args: {
    message: {
      type: 'user',
      text: 'Please rerun the example check.',
    },
  },
}

export const QueuedPromptIsDimmed: Story = {
  name: 'A queued prompt is labeled and dimmed',
  args: {
    message: {
      type: 'queued_user',
      content: [{ type: 'text', text: 'Please rerun the fixture check.' }],
    },
  },
}

export const QueuedPromptShowsItsImage: Story = {
  name: 'A queued prompt shows its dimmed image attachment',
  args: {
    message: {
      type: 'queued_user',
      content: [
        { type: 'text', text: 'Inspect this screenshot: ' },
        {
          type: 'image',
          source: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
          available: true,
        },
      ],
    },
  },
}

export const NonImageAttachmentNameAppears: Story = {
  name: 'The user message shows the name of a document attachment',
  args: {
    message: {
      type: 'user_with_content',
      content: [
        { type: 'text', text: 'Inspect this document.' },
        { type: 'file', name: 'sample-layout.html' },
      ],
    },
  },
}
