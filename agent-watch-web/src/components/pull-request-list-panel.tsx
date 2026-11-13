import '#components/pull-request-list-panel.css'

import { useState } from 'react'

import { CompletedItemsToggle } from '#components/completed-items-toggle'
import { PullRequestMergeConfirmation } from '#components/pull-request-merge-confirmation'
import { useSessionOperations } from '#session-operation-context'
import type { SessionOperation } from '#session-operations'
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
  const [showCompleted, setShowCompleted] = useState(false)
  const [pendingMerge, setPendingMerge] = useState<SessionPullRequest | null>(
    () =>
      pullRequests.find(
        (pullRequest) => pullRequest.number === confirmingPullRequestNumber,
      ) ?? null,
  )
  const { pendingOperations, run } = useSessionOperations()
  const mergeOperation: SessionOperation | null =
    pendingMerge === null ||
    pendingMerge.owner === null ||
    pendingMerge.repo === null ||
    pendingMerge.headSha === null
      ? null
      : {
          type: 'merge',
          sessionId,
          number: pendingMerge.number,
          owner: pendingMerge.owner,
          repo: pendingMerge.repo,
          expectedHeadSha: pendingMerge.headSha,
        }
  const merging =
    mergeOperation !== null &&
    pendingOperations.some(
      ({ operation }) =>
        operation.type === 'merge' &&
        operation.sessionId === sessionId &&
        operation.number === mergeOperation.number,
    )
  const [feedback, setFeedback] = useState<{
    kind: 'success' | 'error'
    message: string
  } | null>(null)

  const confirmMerge = async () => {
    if (pendingMerge === null || mergeOperation === null || merging) return
    setFeedback(null)
    const result = await run(mergeOperation)
    result.match(
      (response) => {
        if (!isMergeResponse(response)) {
          setFeedback({
            kind: 'error',
            message: 'The server returned an invalid merge response.',
          })
          return
        }
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

  const activePullRequests = pullRequests.filter(
    (pullRequest) =>
      pullRequest.state !== 'Merged' && pullRequest.state !== 'Closed',
  )
  const completedPullRequests = pullRequests.filter(
    (pullRequest) =>
      pullRequest.state === 'Merged' || pullRequest.state === 'Closed',
  )
  const visiblePullRequests = showCompleted
    ? [...activePullRequests, ...completedPullRequests]
    : activePullRequests

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
        {visiblePullRequests.map((pullRequest) => (
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
            {pullRequest.state === 'Open' &&
            pullRequest.headSha !== null &&
            pullRequest.owner !== null &&
            pullRequest.repo !== null ? (
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
      {completedPullRequests.length === 0 ? null : (
        <CompletedItemsToggle
          count={completedPullRequests.length}
          expanded={showCompleted}
          onToggle={() => {
            setShowCompleted((expanded) => !expanded)
          }}
          placement="pull-request-list"
        />
      )}
      {pendingMerge === null ? null : (
        <PullRequestMergeConfirmation
          number={pendingMerge.number}
          headSha={pendingMerge.headSha ?? ''}
          merging={merging}
          onCancel={() => {
            setPendingMerge(null)
          }}
          onConfirm={() => void confirmMerge()}
        />
      )}
    </div>
  )
}
