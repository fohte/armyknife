import { Maximize2, Minimize2 } from 'lucide-react'

import type { ReviewDocumentSummary } from '#components/review-document-summary'

type ReviewDocumentNavigationProps = {
  summary?: ReviewDocumentSummary
  summaries?: readonly ReviewDocumentSummary[]
  title?: string
  isMobile?: boolean
  isExpanded?: boolean
  canSave?: boolean
  busy?: boolean
  onTitleChange?: (title: string) => void
  onSelectDocument?: (id: string) => void
  onToggleExpanded?: () => void
}

function kindLabel(kind: ReviewDocumentSummary['kind']) {
  switch (kind) {
    case 'pr_draft':
      return 'PR draft'
    case 'issue_comment':
      return 'Issue comment'
    case 'issue_body':
      return 'Issue body'
    case 'review_replies':
      return 'Review replies'
    case 'draft':
      return 'Draft'
  }
}

function targetLabel(summary: ReviewDocumentSummary) {
  const { target } = summary
  switch (summary.kind) {
    case 'pr_draft':
      return [target.repository, target.branch].filter(Boolean).join(' · ')
    case 'issue_comment':
    case 'issue_body':
      return [
        target.issueNumber === undefined
          ? null
          : `#${String(target.issueNumber)}`,
        target.title,
      ]
        .filter(Boolean)
        .join(' · ')
    case 'review_replies':
      return [
        target.pullRequestNumber === undefined
          ? null
          : `PR #${String(target.pullRequestNumber)}`,
        `${String(target.threadCount ?? 0)} threads`,
      ]
        .filter(Boolean)
        .join(' · ')
    case 'draft':
      return ''
  }
}

export function ReviewDocumentNavigation({
  summary,
  summaries = [],
  title = '',
  isMobile = false,
  isExpanded = false,
  canSave = true,
  busy = false,
  onTitleChange = () => undefined,
  onSelectDocument = () => undefined,
  onToggleExpanded = () => undefined,
}: ReviewDocumentNavigationProps) {
  return (
    <>
      {summary === undefined ? null : (
        <header className="review-document-header">
          <div className="review-document-heading">
            <div className="review-document-heading-context">
              <span className="review-document-kind">
                {kindLabel(summary.kind)}
              </span>
              <span className="review-document-target">
                {targetLabel(summary)}
              </span>
              <span
                aria-label={summary.isEditor ? 'Editor open' : 'Waiting'}
                className={`review-document-dot${summary.isEditor ? ' review-document-dot--editor' : ''}`}
              />
            </div>
            {summary.kind === 'pr_draft' ? (
              <label className="review-document-title-field">
                <span>Title</span>
                <input
                  aria-label="Draft title"
                  className="review-document-input"
                  disabled={!canSave || busy}
                  value={title}
                  onChange={(event) => {
                    onTitleChange(event.currentTarget.value)
                  }}
                />
              </label>
            ) : null}
          </div>
          {isMobile ? null : (
            <button
              aria-label={
                isExpanded ? 'Exit full width review' : 'Open draft full width'
              }
              aria-pressed={isExpanded}
              className="review-document-expand"
              type="button"
              onClick={onToggleExpanded}
            >
              {isExpanded ? (
                <Minimize2 aria-hidden="true" size={16} />
              ) : (
                <Maximize2 aria-hidden="true" size={16} />
              )}
            </button>
          )}
        </header>
      )}
      {summaries.length === 0 ? null : (
        <nav className="review-document-tabs" aria-label="Open drafts">
          {summaries.map((item) => (
            <button
              key={item.id}
              aria-label={`${kindLabel(item.kind)} ${targetLabel(item)}`.trim()}
              aria-current={item.id === summary?.id ? 'page' : undefined}
              className={`review-document-tab${item.id === summary?.id ? ' review-document-tab--selected' : ''}`}
              type="button"
              onClick={() => {
                onSelectDocument(item.id)
              }}
            >
              <span className="review-document-tab-kind">
                {kindLabel(item.kind)}
              </span>
              <span className="review-document-tab-target">
                {targetLabel(item)}
              </span>
              <span
                aria-label={item.isEditor ? 'Editor open' : 'Waiting'}
                className={`review-document-dot${item.isEditor ? ' review-document-dot--editor' : ''}`}
              />
            </button>
          ))}
        </nav>
      )}
    </>
  )
}
