import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import { ReviewDocumentPanel } from '#components/review-document-panel'

const desktopViewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="desktop"
    style={{ width: '640px', height: '720px', display: 'flex' }}
  >
    <Story />
  </div>
)

const phoneViewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="phone"
    style={{ width: '390px', height: '844px', display: 'flex' }}
  >
    <Story />
  </div>
)

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
} satisfies Meta<typeof ReviewDocumentPanel>

export default meta
type Story = StoryObj<typeof meta>

export const EditingTheReviewDocument: Story = {
  name: 'One issue comment opens in preview with its metadata',
  args: {
    summary: {
      id: 'fixture-issue-comment',
      kind: 'issue_comment',
      target: {
        issueNumber: 28,
        title: 'Improve the retry policy',
      },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    summaries: [
      {
        id: 'fixture-issue-comment',
        kind: 'issue_comment',
        target: {
          issueNumber: 28,
          title: 'Improve the retry policy',
        },
        isEditor: true,
        canSave: true,
        canFinish: true,
      },
    ],
  },
  decorators: [desktopViewport],
}

export const MultipleDraftsCanBeSelected: Story = {
  name: 'The panel shows several drafts and highlights the open editor',
  args: {
    summary: {
      id: 'fixture-pr-draft',
      kind: 'pr_draft',
      target: { repository: 'example/service', branch: 'retry-policy' },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    summaries: [
      {
        id: 'fixture-pr-draft',
        kind: 'pr_draft',
        target: { repository: 'example/service', branch: 'retry-policy' },
        isEditor: true,
        canSave: true,
        canFinish: true,
      },
      {
        id: 'fixture-issue-comment',
        kind: 'issue_comment',
        target: { issueNumber: 28, title: 'Improve the retry policy' },
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
    frontmatter: { title: 'Improve retries', status: 'draft' },
  },
  decorators: [desktopViewport],
}

export const WaitingDraftCanBeRead: Story = {
  name: 'A queued draft stays readable while it waits for the editor',
  args: {
    summary: {
      id: 'fixture-queued-draft',
      kind: 'draft',
      target: {},
      isEditor: false,
      canSave: false,
      canFinish: false,
    },
    summaries: [
      {
        id: 'fixture-queued-draft',
        kind: 'draft',
        target: {},
        isEditor: false,
        canSave: false,
        canFinish: false,
      },
    ],
    canSave: false,
    canFinish: false,
  },
  decorators: [desktopViewport],
}

export const PullRequestDraftShowsItsTitle: Story = {
  name: 'A pull request draft shows its editable title above the body',
  args: {
    summary: {
      id: 'fixture-pr-draft',
      kind: 'pr_draft',
      target: { repository: 'example/service', branch: 'retry-policy' },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    summaries: [
      {
        id: 'fixture-pr-draft',
        kind: 'pr_draft',
        target: { repository: 'example/service', branch: 'retry-policy' },
        isEditor: true,
        canSave: true,
        canFinish: true,
      },
    ],
    frontmatter: { title: 'Improve retry handling', labels: ['reliability'] },
  },
  decorators: [desktopViewport],
}

export const RawDraftCanBeEdited: Story = {
  name: 'The Edit view shows the raw draft file',
  args: { view: 'edit' },
  decorators: [desktopViewport],
}

export const UnsavedChangesAreVisible: Story = {
  name: 'The review document shows its unsaved change status',
  args: { unsavedChanges: true },
  decorators: [desktopViewport],
}

export const MobileEditing: Story = {
  name: 'The mobile review document shows its editor',
  args: { isMobile: true },
  decorators: [phoneViewport],
}

export const MobilePreview: Story = {
  name: 'The mobile review document shows its Markdown preview',
  args: { view: 'preview', isMobile: true },
  decorators: [phoneViewport],
}

export const SavingChanges: Story = {
  name: 'The edited review document is being saved',
  args: { status: 'saving' },
  decorators: [desktopViewport],
}

export const SavedChanges: Story = {
  name: 'The latest review document changes are saved',
  args: { status: 'saved' },
  decorators: [desktopViewport],
}

export const SaveFailed: Story = {
  name: 'The review document shows a save error',
  args: { status: 'error', error: 'Could not save the document.' },
  decorators: [desktopViewport],
}

export const SaveRejectedBecauseDocumentChanged: Story = {
  name: 'The review document explains that the draft changed elsewhere',
  args: {
    status: 'error',
    error:
      'This draft changed since you opened it. Your edits are still here; copy them before reloading to see the latest version.',
  },
  decorators: [desktopViewport],
}

export const FinishingReview: Story = {
  name: 'The review is finishing after the document was saved',
  args: { status: 'finishing' },
  decorators: [desktopViewport],
}
