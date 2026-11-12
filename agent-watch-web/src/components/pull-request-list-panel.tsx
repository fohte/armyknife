import '#components/pull-request-list-panel.css'

import { useState } from 'react'

import type { SessionPullRequest } from '#types'

type PullRequestListPanelProps = {
  pullRequests: readonly SessionPullRequest[]
  loading: boolean
  error: string | null
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
  pullRequests,
  loading,
  error,
}: PullRequestListPanelProps) {
  const [showCompleted, setShowCompleted] = useState(false)
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
          </li>
        ))}
      </ul>
      {completedPullRequests.length === 0 ? null : (
        <button
          className="pull-request-list-completed-toggle"
          type="button"
          aria-expanded={showCompleted}
          onClick={() => {
            setShowCompleted((expanded) => !expanded)
          }}
        >
          {showCompleted ? 'Hide' : 'Show'}{' '}
          {String(completedPullRequests.length)} merged
        </button>
      )}
    </div>
  )
}
