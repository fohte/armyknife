import type { Meta, StoryObj } from '@storybook/react-vite'

import { ImageLightbox } from '#components/image-lightbox'

const meta = {
  component: ImageLightbox,
  args: {
    src: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
    alt: 'Sample illustration',
  },
} satisfies Meta<typeof ImageLightbox>

export default meta
type Story = StoryObj<typeof meta>

export const InlineImage: Story = {
  name: 'The image waits for a click before opening the lightbox',
}

export const ExpandedImage: Story = {
  name: 'The image lightbox is open above the page',
  args: { defaultOpen: true },
}
