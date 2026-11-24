import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import { ReviewDocumentPanel } from '#components/review-document-panel'
import type { ReviewDocumentSummary } from '#components/review-document-summary'

const issueComment: ReviewDocumentSummary = {
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
}

const issueBody: ReviewDocumentSummary = {
  ...issueComment,
  id: 'fixture-issue-body',
  kind: 'issue_body',
  isEditor: true,
}

const prDraft: ReviewDocumentSummary = {
  id: 'fixture-pr-draft',
  kind: 'pr_draft',
  target: { repository: 'example/web-app', branch: 'feature/retry-limit' },
  startedAt: Date.now() - 120_000,
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const waitingPrDraft: ReviewDocumentSummary = {
  ...prDraft,
  isEditor: false,
  canSave: false,
  canFinish: false,
}

const reviewReplies: ReviewDocumentSummary = {
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
}

const editingReviewReplies: ReviewDocumentSummary = {
  ...reviewReplies,
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const queuedDraft: ReviewDocumentSummary = {
  id: 'fixture-queued-draft',
  kind: 'draft',
  target: { path: 'docs/retry-plan.md' },
  startedAt: Date.now() - 60_000,
  isEditor: false,
  canSave: false,
  canFinish: false,
}

const desktopViewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="desktop"
    style={{ width: '640px', height: '900px', display: 'flex' }}
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
    frontmatter: { submit: false },
    body: `再現手順を足します。

1. 送信先を応答の遅いサーバーにする
2. 通知を 10 件続けて送る
3. 4 件目から届かない`,
  },
} satisfies Meta<typeof ReviewDocumentPanel>

export default meta
type Story = StoryObj<typeof meta>

export const EditingTheReviewDocument: Story = {
  name: 'One issue comment opens in preview with its metadata',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    frontmatter: null,
    body: `<!-- author: sample-reviewer -->\n<!-- createdAt: 2026-11-24T02:40:00Z -->\n<!-- updatedAt: 2026-11-24T02:40:00Z -->\n<!-- id: comment-node -->\n<!-- databaseId: 506 -->\n\nCould we retry the pending delivery after a short delay?`,
  },
  decorators: [desktopViewport],
}

export const IssueBodyShowsTypedMetadata: Story = {
  name: 'An issue draft shows its title and fields above the body',
  args: {
    summary: issueBody,
    summaries: [issueBody],
    frontmatter: {
      title: 'Notifications stop after several deliveries',
      labels: ['bug', 'notifications'],
      assignees: ['sample-maintainer'],
      parentIssue: 'example/web-app#390',
      subIssues: ['example/web-app#413', 'example/web-app#414'],
      submit: false,
      readonly: { number: 412, author: 'sample-reporter' },
    },
    body: `## Summary\n\nThe fourth notification is not delivered.`,
  },
  decorators: [desktopViewport],
}

export const MobileIssueBodyShowsTypedMetadata: Story = {
  name: 'The mobile issue preview shows its title and fields above the body',
  args: {
    ...IssueBodyShowsTypedMetadata.args,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const IssueCommentShowsTheCommentBody: Story = {
  name: 'An issue comment shows its author and rendered message',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    frontmatter: null,
    body: `<!-- author: sample-reviewer -->\n<!-- createdAt: 2026-11-24T02:40:00Z -->\n<!-- updatedAt: 2026-11-24T02:40:00Z -->\n<!-- id: comment-node -->\n<!-- databaseId: 506 -->\n\nCould we retry the pending delivery after a short delay?`,
  },
  decorators: [desktopViewport],
}

export const ReviewRepliesShowSeveralThreads: Story = {
  name: 'Two review threads show their diffs, comments, and replies',
  args: {
    summary: editingReviewReplies,
    summaries: [editingReviewReplies],
    frontmatter: { submit: false },
    body: `<!-- thread: THREAD_sample_1 path: src/preflight.ts:42 author: @hina-k -->\n- [ ] resolve\n<!-- diff -->\n\`\`\`diff\n@@ -39,5 +41,7 @@ export function preflight(msg) {\n   const target = msg.target;\n-  if (!target) return fail('no target');\n+  if (target == null) {\n+    return fail('no target');\n+  }\n\`\`\`\n<!-- /diff -->\n<!-- comment: @hina-k 2026-01-15T10:30:00Z -->\nこの判定は null も通しますか?\n<!-- /comment -->\n通しません。手前で弾いています。\n\n<!-- thread: THREAD_sample_2 path: src/preflight.ts:57 author: @hina-k -->\n- [x] resolve\n<!-- diff -->\n\`\`\`diff\n@@ -55,4 +56,6 @@ export function preflight(msg) {\n   } catch (e) {\n+    log.warn('preflight failed', e);\n+    return fail(String(e));\n   }\n\`\`\`\n<!-- /diff -->\n<!-- comment: @hina-k 2026-01-15T10:34:00Z -->\n失敗したときのログはどこに出ますか?\n<!-- /comment -->\n<!-- comment: @ren-o 2026-01-15T11:02:00Z -->\n送信ログと同じ場所だと追いやすいです。\n<!-- /comment -->\n送信ログに理由と一緒に出します。`,
  },
  decorators: [desktopViewport],
}

export const MobileReviewRepliesShowSeveralThreads: Story = {
  name: 'The mobile preview shows multiple review thread cards',
  args: {
    ...ReviewRepliesShowSeveralThreads.args,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const MultipleDraftsCanBeSelected: Story = {
  name: 'The panel shows several drafts and highlights the open editor',
  args: {
    summary: issueComment,
    summaries: [issueComment, waitingPrDraft, reviewReplies, queuedDraft],
    frontmatter: null,
  },
  decorators: [desktopViewport],
}

export const WaitingDraftCanBeRead: Story = {
  name: 'A queued draft stays readable while it waits for the editor',
  args: {
    summary: queuedDraft,
    summaries: [queuedDraft],
    canSave: false,
    canFinish: false,
  },
  decorators: [desktopViewport],
}

export const PullRequestDraftShowsItsTitle: Story = {
  name: 'A pull request draft shows its editable title above the body',
  args: {
    summary: prDraft,
    summaries: [prDraft],
    frontmatter: {
      title: '再送の上限を設定で変えられるようにする',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: `## Why

再送の上限が 3 回で固定されていて、応答の遅い送信先では足りない。

## What

- 上限を \`RETRY_LIMIT\` で変えられるようにする
- 既定値は 3 回のまま`,
  },
  decorators: [desktopViewport],
}

export const RawDraftCanBeEdited: Story = {
  name: 'The Edit view shows the raw draft file',
  args: { summary: issueComment, summaries: [issueComment], view: 'edit' },
  decorators: [desktopViewport],
}

export const UnsavedChangesAreVisible: Story = {
  name: 'The review document shows its unsaved change status',
  args: { unsavedChanges: true },
  decorators: [desktopViewport],
}

export const MobileEditing: Story = {
  name: 'The mobile review document shows its editor',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    view: 'edit',
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const MobilePreview: Story = {
  name: 'The mobile review document shows its Markdown preview',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    view: 'preview',
    isMobile: true,
  },
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
