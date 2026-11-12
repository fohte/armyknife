import '#components/pull-request-list-panel.css'

import { useState } from 'react'

import { requestActionJson } from '#components/conversation-requests'
import type { SessionPullRequest } from '#types'

type PullRequestListPanelProps = {
  sessionId: string
  pullRequests: readonly SessionPullRequest[]
  loading: boolean
  error: string | null
  onRefresh: () => void
  confirmingPullRequestNumber?: number
}

type MergeResponse =
  | { result: 'merged' }
  | { result: 'alreadyMerged' }
  | { result: 'enqueued'; message: string }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isMergeResponse(value: unknown): value is MergeResponse {
  if (!isRecord(value)) return false
  const result = value['result']
  return (
    result === 'merged' ||
    result === 'alreadyMerged' ||
    (result === 'enqueued' && typeof value['message'] === 'string')
  )
}

function stateLabel(state: SessionPullRequest['state']) {
  if (state === 'Open') return 'Open'
  if (state === 'Merged') return 'Merged'
  if (state === 'Closed') return 'Closed'
  return 'Unknown'
}

function ciLabel(state: SessionPullRequest['ciState']) {
  if (state === 'Successful') return 'Passing'
  if (state === 'InProgress') return 'Running'
  if (state === 'Waiting') return 'Waiting'
  return 'Unknown'
}

function reviewLabel(status: SessionPullRequest['reviewStatus']) {
  switch (status) {
    case 'done':
      return 'Done'
    case 'user':
      return 'Needs review'
    case 'working':
      return 'Reviewing'
    case 'waiting':
      return 'Waiting'
    case 'not_started':
      return 'Not started'
    default:
      return 'Unknown'
  }
}

export function PullRequestListPanel({
  sessionId,
  pullRequests,
  loading,
  error,
  onRefresh,
  confirmingPullRequestNumber,
}: PullRequestListPanelProps) {
  const [pendingMerge, setPendingMerge] = useState<SessionPullRequest | null>(
    () =>
      pullRequests.find(
        (pullRequest) => pullRequest.number === confirmingPullRequestNumber,
      ) ?? null,
  )
  const [merging, setMerging] = useState(false)
  const [feedback, setFeedback] = useState<{
    kind: 'success' | 'error'
    message: string
  } | null>(null)

  const confirmMerge = async () => {
    if (pendingMerge === null || merging || pendingMerge.headSha === null)
      return
    setMerging(true)
    setFeedback(null)
    await requestActionJson(
      sessionId,
      `pull-requests/${String(pendingMerge.number)}/merge`,
      { expectedHeadSha: pendingMerge.headSha },
      isMergeResponse,
    ).match(
      (response) => {
        const message =
          response.result === 'merged'
            ? `PR #${String(pendingMerge.number)} merged.`
            : response.result === 'alreadyMerged'
              ? `PR #${String(pendingMerge.number)} was already merged.`
              : `PR #${String(pendingMerge.number)} was added to the merge queue: ${response.message}`
        setFeedback({ kind: 'success', message })
        if (response.result !== 'enqueued') onRefresh()
      },
      (mergeError) => {
        setFeedback({ kind: 'error', message: mergeError.message })
      },
    )
    setPendingMerge(null)
    setMerging(false)
  }

  if (error !== null && pullRequests.length === 0) {
    return (
      <div className="pull-request-list-feedback" role="alert">
        {error}
      </div>
    )
  }
  if (pullRequests.length === 0) {
    return (
      <div className="pull-request-list-feedback" role="status">
        {loading ? 'Loading pull requests…' : 'No linked pull requests.'}
      </div>
    )
  }

  return (
    <div className="pull-request-list-panel">
      {feedback === null ? null : (
        <div
          className={`pull-request-list-merge-feedback pull-request-list-merge-feedback--${feedback.kind}`}
          role={feedback.kind === 'error' ? 'alert' : 'status'}
        >
          {feedback.message}
        </div>
      )}
      {error === null ? null : (
        <div className="pull-request-list-error" role="alert">
          {error}
        </div>
      )}
      <ul className="pull-request-list">
        {pullRequests.map((pullRequest) => (
          <li
            className="pull-request-list-item"
            key={`${pullRequest.url}-${String(pullRequest.number)}`}
          >
            <a
              className="pull-request-list-title"
              href={pullRequest.url}
              target="_blank"
              rel="noreferrer"
            >
              <span className="pull-request-list-number">
                #{String(pullRequest.number)}
              </span>
              <span>{pullRequest.title}</span>
            </a>
            <div className="pull-request-list-meta">
              <span
                className={`pull-request-list-state pull-request-list-state--${stateLabel(pullRequest.state).toLowerCase()}`}
              >
                {stateLabel(pullRequest.state)}
              </span>
              <span>CI: {ciLabel(pullRequest.ciState)}</span>
              <span>Review: {reviewLabel(pullRequest.reviewStatus)}</span>
            </div>
            {pullRequest.state === 'Open' && pullRequest.headSha !== null ? (
              <button
                className="pull-request-list-merge-button"
                type="button"
                disabled={merging}
                onClick={() => {
                  setFeedback(null)
                  setPendingMerge(pullRequest)
                }}
              >
                Merge
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      {loading ? (
        <div className="pull-request-list-refreshing" role="status">
          Refreshing…
        </div>
      ) : null}
      {pendingMerge === null ? null : (
        <div className="pull-request-list-confirm-backdrop">
          <section
            className="pull-request-list-confirm"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="pull-request-merge-title"
          >
            <h3 id="pull-request-merge-title">Merge this pull request?</h3>
            <p>
              Merge PR #{String(pendingMerge.number)} using the confirmed head
              commit?
            </p>
            <code>{pendingMerge.headSha}</code>
            <div className="pull-request-list-confirm-actions">
              <button
                type="button"
                disabled={merging}
                onClick={() => {
                  setPendingMerge(null)
                }}
              >
                Cancel
              </button>
              <button
                type="button"
                disabled={merging}
                onClick={() => void confirmMerge()}
              >
                {merging ? 'Merging…' : 'Confirm merge'}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  )
}
