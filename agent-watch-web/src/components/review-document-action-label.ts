import type { ReviewDocumentSummary } from '#components/review-document-summary'
import type { ReviewDocumentAction } from '#session-operation-types'

export function reviewDocumentActionLabel(
  action: ReviewDocumentAction,
  summary?: Pick<ReviewDocumentSummary, 'target'>,
) {
  switch (action) {
    case 'post_comment':
      return 'Post comment'
    case 'create_pr':
      return summary?.target.pullRequestNumber !== undefined
        ? `Update PR #${String(summary.target.pullRequestNumber)}`
        : summary?.target.pullRequestLookupUnavailable === true
          ? 'Submit PR'
          : 'Create PR'
    case 'post_replies':
      return 'Post replies'
    case 'update_issue':
      return 'Update issue'
  }
}
