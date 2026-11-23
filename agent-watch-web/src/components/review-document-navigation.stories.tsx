import type { Meta, StoryObj } from '@storybook/react-vite'

import { ReviewDocumentNavigation } from '#components/review-document-navigation'

const issueComment = {
  id: 'fixture-issue-comment',
  kind: 'issue_comment',
  target: {
    repository: 'example/web-app',
    issueNumber: 412,
    title: '配信が 4 件目から届かない',
  },
  startedAt: Date.now() - 180_000,
  isEditor: true,
  canSave: true,
  canFinish: true,
} as const

const prDraft = {
  id: 'fixture-pr-draft',
  kind: 'pr_draft',
  target: { repository: 'example/web-app', branch: 'feature/retry-limit' },
  startedAt: Date.now() - 120_000,
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
  name: 'One draft uses the heading without a draft tab row',
}

export const SeveralDraftsAreVisible: Story = {
  name: 'Several drafts show their targets and editor status',
  args: {
    summary: issueComment,
    summaries: [
      issueComment,
      {
        ...prDraft,
        isEditor: false,
        canSave: false,
        canFinish: false,
      },
      {
        id: 'fixture-review-replies',
        kind: 'review_replies',
        target: {
          repository: 'example/web-app',
          pullRequestNumber: 2049,
          threadCount: 2,
        },
        startedAt: Date.now() - 60_000,
        isEditor: false,
        canSave: false,
        canFinish: false,
      },
      {
        id: 'fixture-queued-draft',
        kind: 'draft',
        target: { path: 'docs/retry-plan.md' },
        startedAt: Date.now() - 60_000,
        isEditor: false,
        canSave: false,
        canFinish: false,
      },
    ],
  },
}
