import type { Meta, StoryObj } from '@storybook/react-vite'

import { ReviewDocumentIssueReference } from '#components/review-document-issue-reference'

const meta = {
  component: ReviewDocumentIssueReference,
  args: { reference: 'sample-owner/sample-repo#42' },
  decorators: [
    (Story) => (
      <div
        className="review-document-issue-reference"
        style={{ width: '360px' }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ReviewDocumentIssueReference>

export default meta
type Story = StoryObj<typeof meta>

export const WithTitle: Story = {
  name: 'A linked issue shows its title beside the reference',
  args: { title: 'Sample issue title' },
}

export const WithoutTitle: Story = {
  name: 'A linked issue keeps its reference when the title is unavailable',
  args: { title: undefined },
}
