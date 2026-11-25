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
  target: {
    repository: 'example/web-app',
    issueNumber: 418,
    title: '設定画面から再送の上限を変えられるようにする',
  },
  startedAt: Date.now() - 60_000,
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

const japaneseRepoPrDraft: ReviewDocumentSummary = {
  ...prDraft,
  target: { repository: 'example/handbook', branch: 'feature/retry-limit' },
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

const editableReviewReplies: ReviewDocumentSummary = {
  ...reviewReplies,
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const newIssueDraft: ReviewDocumentSummary = {
  id: 'fixture-new-issue',
  kind: 'draft',
  target: { repository: 'example/web-app' },
  startedAt: Date.now() - 60_000,
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const issueCommentBody = `再現手順を足します。

1. 送信先を応答の遅いサーバーにする
2. 通知を 10 件続けて送る
3. 4 件目から届かない`

const japanesePrDraftBody = `## Why

再送の上限が 3 回で固定されていて、応答の遅い送信先では足りない。

## What

- 上限を設定で変えられるようにする
- 既定値は 3 回のまま`

const englishPrDraftBody = `## Why

The retry limit is fixed at 3, which is too few for slow destinations.

## What

- Make the limit configurable with the retry setting`

const reviewRepliesBody = `<!-- thread: THREAD_sample_1 path: src/preflight.ts:42 author: @sample-assignee -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -39,5 +39,7 @@ export function preflight(msg) {
   const target = msg.target;
-  if (!target) return fail('no target');
>+ if (target == null) {
+    return fail('no target');
+  }
\`\`\`
<!-- /diff -->
<!-- comment: @sample-assignee 2026-01-15T10:30:00Z -->
この判定は null も通しますか?
<!-- /comment -->
通しません。対象の処理より前に検証しています。
空値はそこで除外し、有効な値だけ後続へ渡します。
関連する境界値もテストで確認します。

<!-- thread: THREAD_sample_2 path: src/preflight.ts:57 author: @sample-assignee -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -55,4 +57,6 @@ export function preflight(msg) {
   } catch (e) {
>+    log.warn('preflight failed', e);
+    return fail(String(e));
   }
\`\`\`
<!-- /diff -->
<!-- comment: @sample-assignee 2026-01-15T10:34:00Z -->
失敗したときのログはどこに出ますか?
<!-- /comment -->
送信ログに理由と一緒に出します。
原因を追うときに、送信ログだけで発生箇所まで確認できます。`

const issueBodyText = `## 背景

再送の上限が 3 回で固定されていて、応答の遅い送信先では足りない。

## やること

- 設定画面に再送の上限の入力欄を足す
- 既定値は 3 回のまま`

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

const designComparisonViewport = (Story: ComponentType) => (
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
    body: issueCommentBody,
  },
} satisfies Meta<typeof ReviewDocumentPanel>

export default meta
type Story = StoryObj<typeof meta>

const japaneseRepoPrDraftStoryArgs = {
  summary: japaneseRepoPrDraft,
  summaries: [japaneseRepoPrDraft],
  view: 'preview',
  frontmatter: {
    title: '再送の上限を設定で変えられるようにする',
    steps: { submit: false },
  },
  body: japanesePrDraftBody,
} satisfies Story['args']

const englishRepoPrDraftStoryArgs = {
  summary: prDraft,
  summaries: [prDraft],
  view: 'preview',
  frontmatter: {
    title: 'Make the retry limit configurable',
    steps: { 'ready-for-translation': false, submit: false },
  },
  body: englishPrDraftBody,
} satisfies Story['args']

const issueCommentActionStoryArgs = {
  summary: issueComment,
  summaries: [issueComment],
  frontmatter: { submit: false },
  body: issueCommentBody,
} satisfies Story['args']

const reviewRepliesActionStoryArgs = {
  summary: editableReviewReplies,
  summaries: [editableReviewReplies],
  frontmatter: { submit: false },
  body: reviewRepliesBody,
} satisfies Story['args']

const issueBodyActionStoryArgs = {
  summary: issueBody,
  summaries: [],
  view: 'preview',
  frontmatter: {
    title: '再送上限を変更する',
    submit: false,
  },
  body: issueBodyText,
} satisfies Story['args']

export const EditingTheReviewDocument: Story = {
  name: 'The editor shows an issue comment as raw Markdown',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    frontmatter: null,
    view: 'edit',
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
      title: '設定画面から再送の上限を変えられるようにする',
      labels: ['enhancement', 'settings'],
      assignees: ['sample-assignee'],
      parentIssue: 'example/web-app#400',
      subIssues: ['example/web-app#419', 'example/web-app#420'],
      submit: false,
      readonly: { number: 418, author: 'sample-reporter' },
    },
    relatedIssueTitles: {
      'example/web-app#400': '遅延した通知を再送する',
      'example/web-app#419': '保留中の通知配信を再試行する',
      'example/web-app#420': '通知配信の試行回数を記録する',
    },
    body: issueBodyText,
  },
  decorators: [desktopViewport],
}

export const NewIssueDraftShowsRelatedTitles: Story = {
  name: 'A new issue draft shows titles beside its related references',
  args: {
    summary: newIssueDraft,
    summaries: [newIssueDraft],
    frontmatter: {
      title: 'Sample issue title',
      parentIssue: 'sample-owner/sample-repo#42',
      subIssues: ['sample-owner/sample-repo#43', 'sample-owner/sample-repo#44'],
      submit: false,
    },
    relatedIssueTitles: {
      'sample-owner/sample-repo#42': 'Sample parent issue title',
      'sample-owner/sample-repo#43': 'Sample first sub-issue title',
      'sample-owner/sample-repo#44': 'Sample second sub-issue title',
    },
    body: 'Sample issue body.',
  },
  decorators: [desktopViewport],
}

export const MobileNewIssueDraftShowsRelatedTitles: Story = {
  name: 'The mobile new issue draft shows titles beside its related references',
  args: {
    ...NewIssueDraftShowsRelatedTitles.args,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const IssueBodyEditShowsMilestone: Story = {
  name: 'An issue body keeps its milestone visible in Edit mode',
  args: {
    summary: issueBody,
    summaries: [issueBody],
    frontmatter: {
      title: '設定画面から再送の上限を変えられるようにする',
      labels: ['enhancement', 'settings'],
      assignees: ['sample-assignee'],
      parentIssue: 'example/web-app#400',
      subIssues: ['example/web-app#419', 'example/web-app#420'],
      milestone: '2026 Q4',
      submit: false,
      readonly: { number: 418, author: 'sample-reporter' },
    },
    body: issueBodyText,
    view: 'edit',
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
  name: 'An issue comment shows its rendered body',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    frontmatter: null,
    body: `<!-- author: sample-reviewer -->\n<!-- createdAt: 2026-11-24T02:40:00Z -->\n<!-- updatedAt: 2026-11-24T02:40:00Z -->\n<!-- id: comment-node -->\n<!-- databaseId: 506 -->\n\n再現手順を足します。\n\n· 1. 送信先を応答の遅いサーバーにする\n\n· 2. 通知を 10 件続けて送る\n\n· 3. 4 件目から届かない`,
  },
  decorators: [desktopViewport],
}

export const ReviewRepliesShowSeveralThreads: Story = {
  name: 'Two review threads show their diffs, comments, and replies',
  args: {
    summary: editableReviewReplies,
    summaries: [editableReviewReplies],
    frontmatter: { submit: false },
    body: `<!-- thread: THREAD_sample_1 path: src/preflight.ts:42 author: @sample-assignee -->\n- [ ] resolve\n<!-- diff -->\n\`\`\`diff\n@@ -39,5 +39,7 @@ export function preflight(msg) {\n   const target = msg.target;\n-  if (!target) return fail('no target');\n>+  if (target == null) {\n+    return fail('no target');\n+  }\n\`\`\`\n<!-- /diff -->\n<!-- comment: @sample-assignee 2026-01-15T10:30:00Z -->\nこの判定は null も通しますか?\n<!-- /comment -->\n通しません。手前で弾いています。\n\n<!-- thread: THREAD_sample_2 path: src/preflight.ts:57 author: @sample-assignee -->\n- [x] resolve\n<!-- diff -->\n\`\`\`diff\n@@ -55,4 +57,6 @@ export function preflight(msg) {\n   } catch (e) {\n>+    log.warn('preflight failed', e);\n+    return fail(String(e));\n   }\n\`\`\`\n<!-- /diff -->\n<!-- comment: @sample-assignee 2026-01-15T10:34:00Z -->\n失敗したときのログはどこに出ますか?\n<!-- /comment -->\n<!-- comment: @ren-o 2026-01-15T11:02:00Z -->\n送信ログと同じ場所だと追いやすいです。\n<!-- /comment -->\n送信ログに理由と一緒に出します。`,
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

export const NewIssueDraftCanBeApproved: Story = {
  name: 'A new issue draft keeps its approval field available',
  args: {
    summary: newIssueDraft,
    summaries: [newIssueDraft],
    view: 'preview',
    frontmatter: {
      title: 'Retry jobs after a temporary outage',
      labels: ['bug'],
      submit: false,
    },
    body: 'Describe the issue and expected behavior.',
  },
  decorators: [desktopViewport],
}

export const PullRequestDraftShowsItsTitle: Story = {
  name: 'A pull request draft shows its editable title above the body',
  args: {
    summary: prDraft,
    summaries: [prDraft],
    view: 'edit',
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

export const EnglishRepoDraftAwaitsTranslation: Story = {
  name: 'The Japanese pull request draft awaits translation approval',
  args: {
    summary: prDraft,
    summaries: [prDraft],
    view: 'edit',
    frontmatter: {
      title: '再送の上限を設定で変えられるようにする',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: `## Why\n\n再送の上限が 3 回で固定されていて、応答の遅い送信先では足りない。\n\n## What\n\n- 上限を \`RETRY_LIMIT\` で変えられるようにする\n- 既定値は 3 回のまま`,
  },
  decorators: [designComparisonViewport],
}

export const MobileEnglishRepoDraftAwaitsTranslation: Story = {
  name: 'The iPhone panel shows the translation approval step',
  args: {
    summary: prDraft,
    summaries: [prDraft],
    isMobile: true,
    view: 'edit',
    frontmatter: {
      title: '再送の上限を設定で変えられるようにする',
      steps: { 'ready-for-translation': false, submit: false },
    },
    body: `## Why\n\n再送の上限が 3 回で固定されていて、応答の遅い送信先では足りない。\n\n## What\n\n- 上限を \`RETRY_LIMIT\` で変えられるようにする\n- 既定値は 3 回のまま`,
  },
  decorators: [phoneViewport],
}

export const JapaneseRepoDraftCanCreatePrImmediately: Story = {
  name: 'A Japanese repository draft can create its PR immediately.',
  args: japaneseRepoPrDraftStoryArgs,
  decorators: [designComparisonViewport],
}

export const MobileJapaneseRepoDraftCanCreatePrImmediately: Story = {
  name: 'The iPhone Japanese repository draft can create its PR immediately.',
  args: {
    ...japaneseRepoPrDraftStoryArgs,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const EnglishRepoDraftAwaitsPrCreation: Story = {
  name: 'The translated pull request draft is ready to create',
  args: englishRepoPrDraftStoryArgs,
  decorators: [designComparisonViewport],
}

export const MobileEnglishRepoDraftAwaitsPrCreation: Story = {
  name: 'The iPhone English PR draft is ready to create.',
  args: {
    ...englishRepoPrDraftStoryArgs,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const CommentCanBePostedImmediately: Story = {
  name: 'An issue comment can be posted or sent back',
  args: issueCommentActionStoryArgs,
  decorators: [designComparisonViewport],
}

export const MobileCommentCanBePostedImmediately: Story = {
  name: 'The iPhone issue comment can be posted or sent back.',
  args: {
    ...issueCommentActionStoryArgs,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const RepliesCanBePostedImmediately: Story = {
  name: 'A PR review reply preview fills the desktop panel.',
  args: reviewRepliesActionStoryArgs,
  decorators: [desktopViewport],
}

export const ReviewReplyEditFillsDesktopPanel: Story = {
  name: 'A PR review reply draft fills the desktop editor.',
  args: {
    ...reviewRepliesActionStoryArgs,
    view: 'edit',
  },
  decorators: [desktopViewport],
}

export const MobileReviewReplyEditFillsPanel: Story = {
  name: 'The iPhone PR review reply draft fills the editor.',
  args: {
    ...reviewRepliesActionStoryArgs,
    isMobile: true,
    view: 'edit',
  },
  decorators: [phoneViewport],
}

export const MobileRepliesCanBePostedImmediately: Story = {
  name: 'The iPhone PR review reply preview fills the panel.',
  args: {
    ...reviewRepliesActionStoryArgs,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const IssueCanBeUpdatedImmediately: Story = {
  name: 'An issue body can be updated or sent back',
  args: issueBodyActionStoryArgs,
  decorators: [designComparisonViewport],
}

export const MobileIssueCanBeUpdatedImmediately: Story = {
  name: 'The iPhone issue body can be updated or sent back.',
  args: {
    ...issueBodyActionStoryArgs,
    isMobile: true,
  },
  decorators: [phoneViewport],
}

export const PostingTheComment: Story = {
  name: 'The issue comment button shows that posting is in progress',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    frontmatter: { submit: false },
    status: 'submitting',
    pendingAction: 'post_comment',
  },
  decorators: [desktopViewport],
}

export const PostingTheCommentFailed: Story = {
  name: 'The issue comment panel keeps the draft after a posting error',
  args: {
    summary: issueComment,
    summaries: [issueComment],
    frontmatter: { submit: false },
    status: 'error',
    error: 'Could not post the comment.',
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
