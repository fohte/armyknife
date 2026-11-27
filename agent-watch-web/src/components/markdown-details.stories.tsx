import '#components/markdown-message.css'

import type { Meta, StoryObj } from '@storybook/react-vite'

import { MarkdownDetails } from '#components/markdown-details'

type MarkdownDetailsStoryArgs = {
  body: string
  open: boolean
  summary: string
}

const meta = {
  title: 'Components/MarkdownDetails',
  args: {
    body: 'The body starts expanded and can be collapsed from its summary row.',
    open: false,
    summary: 'Additional information',
  },
  render: ({ body, open, summary }) => (
    <div className="markdown-message">
      <MarkdownDetails open={open}>
        <span className="markdown-details-summary-content">{summary}</span>
        <p>{body}</p>
      </MarkdownDetails>
    </div>
  ),
} satisfies Meta<MarkdownDetailsStoryArgs>

export default meta
type Story = StoryObj<typeof meta>

export const Collapsed: Story = {
  name: 'The summary row shows while the body is hidden',
}

export const Expanded: Story = {
  name: 'The summary row shows the expanded body',
  args: { open: true },
}
