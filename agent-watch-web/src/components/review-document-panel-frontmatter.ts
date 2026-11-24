import type { ReviewDocumentSummary } from '#components/review-document-summary'
import type { JsonValue } from '#types'

type JsonObject = { [key: string]: JsonValue }

function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function getReviewDocumentFrontmatterEntries(
  frontmatter: JsonValue,
  summary: ReviewDocumentSummary | undefined,
): [string, JsonValue][] | null {
  if (!isJsonObject(frontmatter)) return null

  return Object.entries(frontmatter).filter(([name]) => {
    if (summary?.kind === 'issue_body' || summary?.kind === 'review_replies') {
      return (
        name === 'submit' ||
        (summary.kind === 'issue_body' && name === 'milestone')
      )
    }
    if (
      summary?.kind === 'pr_draft' &&
      (name === 'title' || name === 'steps')
    ) {
      return false
    }
    if (name === 'submit') {
      return (
        summary?.kind === 'draft' && summary.target.repository !== undefined
      )
    }
    return true
  })
}
