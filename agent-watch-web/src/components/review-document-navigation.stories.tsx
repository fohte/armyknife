import type { Meta, StoryObj } from '@storybook/react-vite'

import { ReviewDocumentNavigation } from '#components/review-document-navigation'

const issueComment = {
  id: 'fixture-issue-comment',
  kind: 'issue_comment',
  target: { issueNumber: 28, title: 'Improve retry handling' },
  isEditor: true,
  canSave: true,
  canFinish: true,
} as const

const prDraft = {
  id: 'fixture-pr-draft',
  kind: 'pr_draft',
  target: { repository: 'example/api', branch: 'retry-window' },
  isEditor: true,
  canSave: true,
  canFinish: true,
} as const

const meta = {
  component: ReviewDocumentNavigation,
  args: {
    summary: issueComment,
    summaries: [issueComment],
  },
} satisfies Meta<typeof ReviewDocumentNavigation>

export default meta
type Story = StoryObj<typeof meta>

export const OneDraftIsSelected: Story = {
  name: 'One draft appears in the row below Review',
}

export const SeveralDraftsAreVisible: Story = {
  name: 'Several drafts show their targets and editor status',
  args: {
    summary: prDraft,
    summaries: [
      prDraft,
      {
        id: 'fixture-issue-comment',
        kind: 'issue_comment',
        target: { issueNumber: 28, title: 'Improve retry handling' },
        isEditor: false,
        canSave: false,
        canFinish: false,
      },
      {
        id: 'fixture-review-replies',
        kind: 'review_replies',
        target: { pullRequestNumber: 15, threadCount: 2 },
        isEditor: false,
        canSave: false,
        canFinish: false,
      },
    ],
    title: 'Improve retry handling',
  },
}

export const PhoneHidesFullWidthControl: Story = {
  name: 'The phone header keeps its title without a full width control',
  args: {
    isMobile: true,
  },
}
