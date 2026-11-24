import '#components/review-document-panel/direct-actions.css'

import type { ReviewDocumentSummary } from '#components/review-document-summary'
import type { ReviewDocumentAction } from '#session-operation-types'
import type { JsonValue } from '#types'

export type { ReviewDocumentAction } from '#session-operation-types'

type ReviewDocumentActionsProps = {
  summary?: ReviewDocumentSummary | undefined
  isMobile: boolean
  canSave: boolean
  canFinish: boolean
  frontmatter: JsonValue
  body: string
  status: 'idle' | 'saving' | 'saved' | 'error' | 'finishing' | 'submitting'
  pendingAction?: ReviewDocumentAction | undefined
  unsavedChanges: boolean
  error: string | null
  onSave: () => void
  onFinish: () => void
  onApproveForTranslation: () => void
  onSubmit: (action: ReviewDocumentAction) => void
}

type JsonObject = Record<string, JsonValue>

function isJsonObject(value: JsonValue): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function ReviewDocumentActions({
  summary,
  isMobile,
  canSave,
  canFinish,
  frontmatter,
  body,
  status,
  pendingAction,
  unsavedChanges,
  error,
  onSave,
  onFinish,
  onApproveForTranslation,
  onSubmit,
}: ReviewDocumentActionsProps) {
  const busy =
    status === 'saving' || status === 'finishing' || status === 'submitting'
  const kind = summary?.kind
  const stepsValue = isJsonObject(frontmatter) ? frontmatter['steps'] : null
  const steps =
    stepsValue !== undefined && isJsonObject(stepsValue) ? stepsValue : null
  const requiresTranslation =
    kind === 'pr_draft' && steps !== null && 'ready-for-translation' in steps
  const draftText = `${
    isJsonObject(frontmatter) && typeof frontmatter['title'] === 'string'
      ? frontmatter['title']
      : ''
  }\n${body}`
  const hasJapanese = /[\u3040-\u30ff\u3400-\u9fff]/u.test(draftText)
  const prStage = hasJapanese ? 1 : 3
  const primaryAction =
    kind === 'issue_comment'
      ? 'post_comment'
      : kind === 'pr_draft'
        ? 'create_pr'
        : kind === 'review_replies'
          ? 'post_replies'
          : kind === 'issue_body'
            ? 'update_issue'
            : null
  const primaryLabel =
    kind === 'issue_comment'
      ? 'Post comment'
      : kind === 'pr_draft'
        ? requiresTranslation && prStage === 1
          ? 'Approve for translation'
          : 'Create PR'
        : kind === 'review_replies'
          ? 'Post replies'
          : kind === 'issue_body'
            ? 'Update issue'
            : 'Finish'
  const actionLabel =
    pendingAction === 'post_comment'
      ? 'Posting comment…'
      : pendingAction === 'create_pr'
        ? 'Creating PR…'
        : pendingAction === 'post_replies'
          ? 'Posting replies…'
          : pendingAction === 'update_issue'
            ? 'Updating issue…'
            : null
  const directHelp =
    kind === 'issue_comment'
      ? `Post comment posts it to #${String(summary?.target.issueNumber ?? '')} right away. Send back returns your edits to the session.`
      : kind === 'pr_draft' && requiresTranslation && prStage === 1
        ? 'The session translates the title and body to English right away. You review the English draft next, then create the PR.'
        : kind === 'pr_draft'
          ? `Create PR creates the PR on ${summary?.target.repository ?? 'the repository'} right away. Send back returns your edits to the session.`
          : kind === 'review_replies'
            ? `Post replies posts them on PR #${String(summary?.target.pullRequestNumber ?? '')} right away. Send back returns your edits to the session.`
            : `Update issue updates it right away. Send back returns your edits to the session.`
  const sendBackTitle =
    kind === 'issue_comment'
      ? 'Close the editor without approving. The session reads your edits and revises the comment.'
      : kind === 'review_replies'
        ? 'Close the editor without approving. The session reads your edits and revises the replies.'
        : 'Close the editor without approving. The session reads your edits and revises the draft.'

  return (
    <footer
      className={`review-document-actions${primaryAction === null ? '' : ' review-document-direct-actions'}`}
    >
      {primaryAction === null ? (
        <>
          <span aria-live="polite" className="review-document-status">
            {status === 'saving'
              ? 'Saving…'
              : status === 'saved'
                ? 'Saved'
                : status === 'finishing'
                  ? 'Finishing review…'
                  : status === 'submitting'
                    ? (actionLabel ?? 'Working…')
                    : (error ?? (unsavedChanges ? 'Unsaved changes' : ''))}
          </span>
          {error === null ? null : (
            <span className="review-document-error" role="alert">
              {error}
            </span>
          )}
          {isMobile ? null : (
            <button disabled={!canSave || busy} type="button" onClick={onSave}>
              Save
            </button>
          )}
          <button
            className="review-document-finish"
            disabled={!canFinish || busy}
            type="button"
            onClick={onFinish}
          >
            Finish
          </button>
        </>
      ) : (
        <>
          {requiresTranslation ? (
            <ol
              aria-label="Steps to create the PR"
              className="review-document-pr-stages"
            >
              {['Review', 'Translate', 'Create PR'].map((label, index) => {
                const current = index === (prStage === 1 ? 0 : 2)
                const done = index < (prStage === 1 ? 0 : 2)
                return (
                  <li
                    aria-current={current ? 'step' : 'false'}
                    key={label}
                    data-current={current}
                    data-complete={done}
                  >
                    <span>{done ? '✓' : String(index + 1)}</span>
                    {label}
                    <span
                      aria-hidden="true"
                      style={{
                        display: index < 2 ? 'inline' : 'none',
                        marginLeft: '2px',
                      }}
                    >
                      —
                    </span>
                  </li>
                )
              })}
            </ol>
          ) : null}
          <div className="review-document-direct-button-row">
            {isMobile ? null : (
              <button
                className="review-document-save"
                disabled={!canSave || busy}
                type="button"
                onClick={onSave}
              >
                Save
              </button>
            )}
            <button
              className="review-document-send-back"
              disabled={!canFinish || busy}
              title={sendBackTitle}
              type="button"
              onClick={onFinish}
            >
              Send back
            </button>
            <button
              className="review-document-finish"
              disabled={!canFinish || busy}
              type="button"
              onClick={() => {
                if (
                  kind === 'pr_draft' &&
                  requiresTranslation &&
                  prStage === 1
                ) {
                  onApproveForTranslation()
                } else {
                  onSubmit(primaryAction)
                }
              }}
            >
              {primaryLabel}
            </button>
          </div>
          <p className="review-document-direct-help">{directHelp}</p>
        </>
      )}
    </footer>
  )
}
