import type { Meta, StoryObj } from '@storybook/react-vite'

import { ReviewDocumentPanel } from '#components/review-document-panel'

const meta = {
  component: ReviewDocumentPanel,
  args: {
    frontmatter: {
      title: 'Retry policy',
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
        style={{
          width: 'min(920px, 100%)',
          minHeight: '500px',
          margin: '0 auto',
          background: '#0a0a0a',
        }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ReviewDocumentPanel>

export default meta
type Story = StoryObj<typeof meta>

export const EditingTheReviewDocument: Story = {
  name: 'The review document is ready to edit and preview',
}

export const MobileEditing: Story = {
  name: 'The mobile review document shows its editor',
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

export const MobilePreview: Story = {
  name: 'The mobile review document shows its Markdown preview',
  args: { mobileView: 'preview' },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
}

export const SavingChanges: Story = {
  name: 'The edited review document is being saved',
  args: { status: 'saving' },
}

export const SavedChanges: Story = {
  name: 'The latest review document changes are saved',
  args: { status: 'saved' },
}

export const SaveFailed: Story = {
  name: 'The review document shows a save error',
  args: { status: 'error', error: 'Could not save the document.' },
}

export const FinishingReview: Story = {
  name: 'The review is finishing after the document was saved',
  args: { status: 'finishing' },
}
