export type ReviewDocumentSummary = {
  id: string
  kind: 'pr_draft' | 'issue_comment' | 'issue_body' | 'review_replies' | 'draft'
  target: {
    repository?: string
    branch?: string
    issueNumber?: number
    pullRequestNumber?: number
    title?: string
    threadCount?: number
  }
  isEditor: boolean
}

export type EditorReviewDocumentState =
  | { status: 'ready'; summary: ReviewDocumentSummary }
  | { status: 'unavailable' }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

export function isReviewDocumentSummaryList(
  value: unknown,
): value is ReviewDocumentSummary[] {
  return (
    Array.isArray(value) &&
    value.every(
      (item) =>
        isRecord(item) &&
        typeof item['id'] === 'string' &&
        (item['kind'] === 'pr_draft' ||
          item['kind'] === 'issue_comment' ||
          item['kind'] === 'issue_body' ||
          item['kind'] === 'review_replies' ||
          item['kind'] === 'draft') &&
        isRecord(item['target']) &&
        (item['target']['repository'] === undefined ||
          typeof item['target']['repository'] === 'string') &&
        (item['target']['branch'] === undefined ||
          typeof item['target']['branch'] === 'string') &&
        (item['target']['issueNumber'] === undefined ||
          typeof item['target']['issueNumber'] === 'number') &&
        (item['target']['pullRequestNumber'] === undefined ||
          typeof item['target']['pullRequestNumber'] === 'number') &&
        (item['target']['title'] === undefined ||
          typeof item['target']['title'] === 'string') &&
        (item['target']['threadCount'] === undefined ||
          typeof item['target']['threadCount'] === 'number') &&
        typeof item['isEditor'] === 'boolean',
    )
  )
}
