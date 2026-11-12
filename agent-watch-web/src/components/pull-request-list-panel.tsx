import '#components/pull-request-list-panel.css'

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
  if (error !== null) {
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
          </li>
        ))}
      </ul>
      {loading ? (
        <div className="pull-request-list-refreshing" role="status">
          Refreshing…
        </div>
      ) : null}
    </div>
  )
}
