import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import { ReviewDocumentFocus } from '#components/review-document-focus'

const desktopViewport = (Story: ComponentType) => (
  <div style={{ width: '800px', height: '900px', display: 'flex' }}>
    <Story />
  </div>
)

const meta = {
  component: ReviewDocumentFocus,
  args: {
    sessionTitle: 'Delivery checks are ready',
    body: `## Summary

Add a clear preview of the draft content.

## Changes

- Keep the source easy to scan
- Show its rendered preview beside it
`,
    onBack: () => undefined,
  },
  decorators: [desktopViewport],
} satisfies Meta<typeof ReviewDocumentFocus>

export default meta
type Story = StoryObj<typeof meta>

export const MarkdownAndPreviewFillTheScreen: Story = {
  name: 'Markdown and Preview fill the screen in two columns',
}
