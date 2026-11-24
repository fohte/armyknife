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

const issueBody: ReviewDocumentSummary = {
  ...issueComment,
  id: 'fixture-issue-body',
  kind: 'issue_body',
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

const reviewRepliesBody = `### src/preflight.ts:42

> この判定は null も通しますか?

通しません。手前で弾いています。

### src/preflight.ts:57

> 失敗したときのログはどこに出ますか?

送信ログに理由と一緒に出します。`

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
    style={{ width: '640px', height: '720px', display: 'flex' }}
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
  name: 'One issue comment opens in preview with its metadata',
  args: {
    summary: issueComment,
    summaries: [issueComment],
  },
  decorators: [desktopViewport],
}

export const MultipleDraftsCanBeSelected: Story = {
  name: 'The panel shows several drafts and highlights the open editor',
  args: {
    summary: issueComment,
    summaries: [issueComment, waitingPrDraft, reviewReplies, queuedDraft],
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
  name: 'Review replies can be posted or sent back',
  args: reviewRepliesActionStoryArgs,
  decorators: [designComparisonViewport],
}

export const MobileRepliesCanBePostedImmediately: Story = {
  name: 'The iPhone review replies can be posted or sent back.',
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
