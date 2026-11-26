import type { ReviewDocumentAction } from '#session-operation-types'

export function reviewDocumentActionLabel(
  action: ReviewDocumentAction,
  pullRequestNumber?: number,
) {
  switch (action) {
    case 'post_comment':
      return 'Post comment'
    case 'create_pr':
      return pullRequestNumber === undefined
        ? 'Create PR'
        : `Update PR #${String(pullRequestNumber)}`
    case 'post_replies':
      return 'Post replies'
    case 'update_issue':
      return 'Update issue'
  }
}
