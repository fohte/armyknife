import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import { ReviewDocumentActions } from '#components/review-document-panel/actions'
import type { ReviewDocumentSummary } from '#components/review-document-summary'

const issueComment: ReviewDocumentSummary = {
  id: 'fixture-issue-comment',
  kind: 'issue_comment',
  target: {
    repository: 'example/service',
    issueNumber: 42,
    title: 'Delivery retries stop too early',
  },
  startedAt: Date.now() - 60_000,
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const prDraft: ReviewDocumentSummary = {
  id: 'fixture-pr-draft',
  kind: 'pr_draft',
  target: { repository: 'example/service', branch: 'feature/retry-limit' },
  startedAt: Date.now() - 60_000,
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const existingPrDraft: ReviewDocumentSummary = {
  ...prDraft,
  target: {
    repository: 'example/service',
    branch: 'feature/retry-limit',
    pullRequestNumber: 73,
  },
}

const prDraftWithUnavailableLookup: ReviewDocumentSummary = {
  ...prDraft,
  target: {
    repository: 'example/service',
    branch: 'feature/retry-limit',
    pullRequestLookupUnavailable: true,
  },
}

const viewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="desktop"
    style={{ width: '640px', minHeight: '120px', display: 'flex' }}
  >
    <Story />
  </div>
)

const meta = {
  component: ReviewDocumentActions,
  args: {
    summary: issueComment,
    isMobile: false,
    canSave: true,
    canFinish: true,
    frontmatter: { submit: false },
    body: 'The fourth delivery is not sent when the destination is slow.',
    status: 'idle',
    unsavedChanges: false,
    error: null,
    onSave: () => undefined,
    onFinish: () => undefined,
    onApproveForTranslation: () => undefined,
    onSubmit: () => undefined,
  },
  decorators: [viewport],
} satisfies Meta<typeof ReviewDocumentActions>

export default meta
type Story = StoryObj<typeof meta>

export const IssueCommentCanBePosted: Story = {
  name: 'An issue comment can be sent back or posted immediately',
}

export const EnglishRepoDraftAwaitsTranslation: Story = {
  name: 'The Japanese pull request draft shows translation as its next step',
  args: {
    summary: prDraft,
    frontmatter: {
      title: 'Make the retry limit configurable',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: '再送の上限を設定で変えられるようにする',
  },
}

export const EnglishRepoDraftCanCreatePr: Story = {
  name: 'The translated pull request draft is ready to create',
  args: {
    summary: prDraft,
    frontmatter: {
      title: 'Make the retry limit configurable',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: 'Make the retry limit configurable for slow destinations.',
  },
}

export const EnglishRepoDraftCanUpdatePr: Story = {
  name: 'The draft updates the existing pull request',
  args: {
    summary: existingPrDraft,
    frontmatter: {
      title: 'Make the retry limit configurable',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: 'Make the retry limit configurable for slow destinations.',
  },
}

export const EnglishRepoDraftHasUnavailablePrLookup: Story = {
  name: 'An unavailable PR lookup keeps the action label neutral',
  args: {
    summary: prDraftWithUnavailableLookup,
    frontmatter: {
      title: 'Make the retry limit configurable',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: 'Make the retry limit configurable for slow destinations.',
  },
}

export const CommentIsPosting: Story = {
  name: 'The action row shows that an issue comment is being posted',
  args: {
    status: 'submitting',
    pendingAction: 'post_comment',
  },
}

export const CommentPostingFailed: Story = {
  name: 'The action row keeps the draft after a posting error',
  args: {
    status: 'error',
    error: 'Could not post the comment.',
  },
}
