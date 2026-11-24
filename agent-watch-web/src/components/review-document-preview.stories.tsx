import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import { ReviewDocumentPreview } from '#components/review-document-preview'
import type { ReviewDocumentSummary } from '#components/review-document-summary'

const commentSummary: ReviewDocumentSummary = {
  id: 'fixture-issue-comment',
  kind: 'issue_comment',
  target: {
    repository: 'example/web-app',
    issueNumber: 412,
    title: 'Notifications stop after several deliveries',
  },
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const repliesSummary: ReviewDocumentSummary = {
  id: 'fixture-review-replies',
  kind: 'review_replies',
  target: {
    repository: 'example/web-app',
    pullRequestNumber: 2049,
    threadCount: 2,
  },
  isEditor: true,
  canSave: true,
  canFinish: true,
}

const viewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="desktop"
    style={{ width: '640px', height: '620px', display: 'flex' }}
  >
    <div style={{ width: '100%', overflow: 'auto', padding: '12px' }}>
      <Story />
    </div>
  </div>
)

const meta = {
  component: ReviewDocumentPreview,
  args: {
    body: 'A rendered Markdown message.',
  },
  decorators: [viewport],
} satisfies Meta<typeof ReviewDocumentPreview>

export default meta
type Story = StoryObj<typeof meta>

export const IssueCommentPreview: Story = {
  name: 'The issue comment shows its author and message',
  args: {
    summary: commentSummary,
    body: `<!-- author: sample-reviewer -->\n<!-- createdAt: 2026-11-24T02:40:00Z -->\n\nCould we retry the pending delivery?`,
  },
}

export const IssueFrontmatterFields: Story = {
  name: 'An issue shows its title and metadata fields',
  args: {
    summary: {
      ...commentSummary,
      id: 'fixture-issue-body',
      kind: 'issue_body',
    },
    mode: 'frontmatter',
    frontmatter: {
      title: 'Notifications stop after several deliveries',
      labels: ['bug', 'notifications'],
      assignees: ['sample-maintainer'],
      parentIssue: 'example/web-app#390',
      subIssues: ['example/web-app#413', 'example/web-app#414'],
    },
  },
}

export const MultipleReviewThreads: Story = {
  name: 'Two review threads show their diff and reply fields',
  args: {
    summary: repliesSummary,
    body: `<!-- thread: THREAD_sample_1 path: src/preflight.ts:42 author: @hina-k -->\n- [ ] resolve\n<!-- diff -->\n\`\`\`diff\n@@ -39,5 +41,7 @@ export function preflight(msg) {\n   const target = msg.target;\n-  if (!target) return fail('no target');\n+  if (target == null) {\n+    return fail('no target');\n+  }\n\`\`\`\n<!-- /diff -->\n<!-- comment: @hina-k 2026-01-15T10:30:00Z -->\nこの判定は null も通しますか?\n<!-- /comment -->\n通しません。手前で弾いています。\n\n<!-- thread: THREAD_sample_2 path: src/preflight.ts:57 author: @hina-k -->\n- [x] resolve\n<!-- diff -->\n\`\`\`diff\n@@ -55,4 +56,6 @@ export function preflight(msg) {\n   } catch (e) {\n+    log.warn('preflight failed', e);\n+    return fail(String(e));\n   }\n\`\`\`\n<!-- /diff -->\n<!-- comment: @hina-k 2026-01-15T10:34:00Z -->\n失敗したときのログはどこに出ますか?\n<!-- /comment -->\n<!-- comment: @ren-o 2026-01-15T11:02:00Z -->\n送信ログと同じ場所だと追いやすいです。\n<!-- /comment -->\n送信ログに理由と一緒に出します。`,
  },
}

export const ReviewMessagesWithoutInlineThreads: Story = {
  name: 'Review messages render when a pull request has no inline threads',
  args: {
    summary: {
      ...repliesSummary,
      target: {
        ...repliesSummary.target,
        threadCount: 0,
      },
    },
    body: `<!-- reviews -->
<!-- review: @sample-reviewer state=COMMENTED 2026-01-15T10:30:00Z -->
The retry policy needs more detail before this change is ready.
<!-- /review -->
<!-- review: @another-reviewer state=COMMENTED 2026-01-15T10:34:00Z -->
Please explain how the retry limit applies to slow destinations.
<!-- /review -->
<!-- /reviews -->`,
  },
}
