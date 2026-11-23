import '#components/review-document-navigation.css'

import { useEffect, useState } from 'react'

import type { ReviewDocumentSummary } from '#components/review-document-summary'

type ReviewDocumentNavigationProps = {
  summary?: ReviewDocumentSummary
  summaries?: readonly ReviewDocumentSummary[]
  title?: string
  canSave?: boolean
  busy?: boolean
  onTitleChange?: (title: string) => void
  onSelectDocument?: (id: string) => void
}

function kindLabel(kind: ReviewDocumentSummary['kind']) {
  switch (kind) {
    case 'pr_draft':
      return 'PR draft'
    case 'issue_comment':
      return 'Issue comment'
    case 'issue_body':
      return 'Issue'
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
        target.repository,
        target.issueNumber === undefined
          ? null
          : `#${String(target.issueNumber)}`,
      ]
        .filter(Boolean)
        .join(' ')
    case 'review_replies':
      return [
        target.repository,
        target.pullRequestNumber === undefined
          ? null
          : `PR #${String(target.pullRequestNumber)}`,
      ]
        .filter(Boolean)
        .join(' ')
    case 'draft':
      return target.path ?? target.repository ?? ''
  }
}

function documentTitle(summary: ReviewDocumentSummary, title: string) {
  if (summary.kind === 'pr_draft') return title
  if (summary.kind === 'issue_comment' || summary.kind === 'issue_body') {
    return [
      summary.target.issueNumber === undefined
        ? null
        : `#${String(summary.target.issueNumber)}`,
      summary.target.title,
    ]
      .filter(Boolean)
      .join(' ')
  }
  if (summary.kind === 'review_replies') {
    return summary.target.title ?? `${targetLabel(summary)} replies`
  }
  const draftTitle = title.length > 0 ? title : summary.target.path
  return draftTitle == null || draftTitle.length === 0 ? 'Draft' : draftTitle
}

function documentDescription(summary: ReviewDocumentSummary) {
  switch (summary.kind) {
    case 'issue_comment':
      return 'New comment on this issue'
    case 'issue_body':
      return 'Edit this issue'
    case 'pr_draft':
      return null
    case 'review_replies':
      return `Replies to ${String(summary.target.threadCount ?? 0)} review threads`
    case 'draft':
      return null
  }
}

function shortTargetLabel(summary: ReviewDocumentSummary) {
  const { target } = summary
  switch (summary.kind) {
    case 'pr_draft':
      return target.branch ?? target.repository ?? ''
    case 'issue_comment':
    case 'issue_body':
      return target.issueNumber === undefined
        ? (target.repository ?? '')
        : `#${String(target.issueNumber)}`
    case 'review_replies':
      return target.pullRequestNumber === undefined
        ? 'Review replies'
        : `PR #${String(target.pullRequestNumber)}`
    case 'draft':
      return target.path?.split('/').at(-1) ?? target.repository ?? 'Draft'
  }
}

function formatElapsed(startedAt: number, now: number) {
  const elapsedMinutes = Math.max(0, Math.floor((now - startedAt) / 60_000))
  if (elapsedMinutes === 0) return 'now'
  if (elapsedMinutes < 60) return `${String(elapsedMinutes)}m`
  const hours = Math.floor(elapsedMinutes / 60)
  const minutes = elapsedMinutes % 60
  return minutes === 0
    ? `${String(hours)}h`
    : `${String(hours)}h ${String(minutes)}m`
}

function editorStatus(summary: ReviewDocumentSummary, now: number) {
  const status = summary.isEditor ? 'In the editor' : 'Queued for the editor'
  return summary.startedAt === undefined
    ? status
    : `${status} · ${formatElapsed(summary.startedAt, now)}`
}

export function ReviewDocumentNavigation({
  summary,
  summaries = [],
  title = '',
  canSave = true,
  busy = false,
  onTitleChange,
  onSelectDocument,
}: ReviewDocumentNavigationProps) {
  const [now, setNow] = useState(Date.now)
  const description =
    summary === undefined ? null : documentDescription(summary)

  useEffect(() => {
    const interval = window.setInterval(() => {
      setNow(Date.now())
    }, 60_000)
    return () => {
      window.clearInterval(interval)
    }
  }, [])

  return (
    <>
      {summaries.length > 1 ? (
        <nav
          aria-label="Open drafts"
          className="review-document-tabs"
          role="tablist"
        >
          {summaries.map((item) => (
            <button
              key={item.id}
              aria-label={[
                `${kindLabel(item.kind)} ${targetLabel(item)}`.trim(),
                item.target.title,
              ]
                .filter(Boolean)
                .join(' · ')}
              aria-selected={item.id === summary?.id}
              className={`review-document-tab${item.id === summary?.id ? ' review-document-tab--selected' : ''}`}
              role="tab"
              type="button"
              onClick={() => {
                onSelectDocument?.(item.id)
              }}
            >
              <span
                aria-hidden="true"
                className={`review-document-dot${item.isEditor ? ' review-document-dot--editor' : ''}`}
              />
              <span className="review-document-tab-kind">
                {kindLabel(item.kind)}
              </span>
              <span className="review-document-tab-target">
                {shortTargetLabel(item)}
              </span>
            </button>
          ))}
        </nav>
      ) : null}
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
              <span className="review-document-editor-status">
                <span
                  aria-hidden="true"
                  className={`review-document-dot${summary.isEditor ? ' review-document-dot--editor' : ''}`}
                />
                {editorStatus(summary, now)}
              </span>
            </div>
            {summary.kind === 'pr_draft' ? (
              <label className="review-document-title-field">
                <span className="review-document-title-label">
                  PR title <code>title</code>
                </span>
                <input
                  aria-label="Draft title"
                  className="review-document-title-input"
                  disabled={!canSave || busy}
                  value={title}
                  onChange={(event) => {
                    onTitleChange?.(event.currentTarget.value)
                  }}
                />
              </label>
            ) : (
              <h2 className="review-document-title">
                {documentTitle(summary, title)}
              </h2>
            )}
            {description === null ? null : (
              <p className="review-document-description">{description}</p>
            )}
          </div>
        </header>
      )}
    </>
  )
}
