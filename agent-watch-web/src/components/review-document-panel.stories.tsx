import type { Meta, StoryObj } from '@storybook/react-vite'

import { ReviewDocumentPanel } from '#components/review-document-panel'

const meta = {
  component: ReviewDocumentPanel,
  args: {
    frontmatter: {
      title: 'Document the retry policy',
      enabled: true,
      labels: ['backend', 'reliability'],
      plan: {
        attempts: 3,
        notifyOnFailure: false,
      },
    },
    body: `## Summary

Describe the retry behavior for temporary service errors.

## Changes

- Retry transient failures
- Stop after three attempts
`,
  },
  decorators: [
    (Story) => (
      <div
        style={{ width: '420px', minHeight: '360px', background: '#0a0a0a' }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ReviewDocumentPanel>

export default meta
type Story = StoryObj<typeof meta>

export const ReviewDocumentIsReadable: Story = {
  name: 'The review shows typed frontmatter and formatted Markdown',
}
