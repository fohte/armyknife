import '#components/session-pull-requests.css'

import { Check, ChevronDown } from 'lucide-react'
import { type ReactNode, useEffect } from 'react'

import type { PullRequest } from '#types'

type SessionPullRequestsProps = {
  pullRequests: readonly PullRequest[]
  repo: string
  isMobile: boolean
  open: boolean
  mobileAction?: ReactNode
  onOpenChange: (open: boolean) => void
}

type PullRequestGroup = {
  name: string
  pullRequests: PullRequest[]
}

function ciStateLabel(ciState: PullRequest['ciState']) {
  switch (ciState) {
    case 'Waiting':
      return 'CI waiting'
    case 'InProgress':
      return 'CI running'
    case 'Successful':
      return 'CI passed'
    case 'Failed':
      return 'CI failed'
    default:
      return 'CI status unknown'
  }
}

function ciStateClass(ciState: PullRequest['ciState']) {
  switch (ciState) {
    case 'Waiting':
    case 'InProgress':
      return 'is-running'
    case 'Successful':
      return 'is-passed'
    case 'Failed':
      return 'is-failed'
    default:
      return 'is-unknown'
  }
}

function pullRequestGroups(pullRequests: readonly PullRequest[]) {
  const groups: PullRequestGroup[] = [
    {
      name: 'This branch',
      pullRequests: pullRequests.filter(
        (pullRequest) => pullRequest.isCurrentBranch === true,
      ),
    },
    {
      name: 'Opened elsewhere',
      pullRequests: pullRequests.filter(
        (pullRequest) =>
          pullRequest.isCurrentBranch !== true &&
          pullRequest.state !== 'Merged',
      ),
    },
    {
      name: 'Merged',
      pullRequests: pullRequests.filter(
        (pullRequest) => pullRequest.state === 'Merged',
      ),
    },
  ]

  return groups.filter((group) => group.pullRequests.length > 0)
}

function pullRequestUrl(pullRequest: PullRequest, fallbackRepo: string) {
  if (pullRequest.url !== undefined) return pullRequest.url
  const repo =
    pullRequest.owner !== undefined && pullRequest.repo !== undefined
      ? `${pullRequest.owner}/${pullRequest.repo}`
      : fallbackRepo
  return `https://github.com/${repo}/pull/${String(pullRequest.number)}`
}

function pullRequestRepo(pullRequest: PullRequest, fallbackRepo: string) {
  if (pullRequest.owner !== undefined && pullRequest.repo !== undefined) {
    return `${pullRequest.owner}/${pullRequest.repo}`
  }
  return fallbackRepo
}

export function SessionPullRequests({
  pullRequests,
  repo,
  isMobile,
  open,
  mobileAction,
  onOpenChange,
}: SessionPullRequestsProps) {
  const currentPullRequest =
    pullRequests.find((pullRequest) => pullRequest.isCurrentBranch === true) ??
    pullRequests.find((pullRequest) => pullRequest.state !== 'Merged') ??
    pullRequests[0]
  const allMerged = pullRequests.every(
    (pullRequest) => pullRequest.state === 'Merged',
  )
  const additionalCount = Math.max(pullRequests.length - 1, 0)
  const triggerLabel =
    currentPullRequest === undefined
      ? ''
      : allMerged
        ? `All ${String(pullRequests.length)} pull requests are merged`
        : `PR #${String(currentPullRequest.number)}, ${currentPullRequest.state.toLowerCase()}, ${ciStateLabel(currentPullRequest.ciState)}${additionalCount > 0 ? `. ${String(additionalCount)} more PR${additionalCount === 1 ? '' : 's'} from this session` : ''}`

  useEffect(() => {
    if (!open) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onOpenChange(false)
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => {
      window.removeEventListener('keydown', closeOnEscape)
    }
  }, [onOpenChange, open])

  if (pullRequests.length === 0 || currentPullRequest === undefined) return null

  const groups = pullRequestGroups(pullRequests)
  const content = (
    <section
      className={`session-pr-list${isMobile ? ' session-pr-list--mobile' : ''}`}
      role="dialog"
      aria-label="Pull requests from this session"
      aria-modal={isMobile || undefined}
      tabIndex={-1}
      onClick={(event) => {
        event.stopPropagation()
      }}
    >
      {isMobile ? (
        <h2 className="session-pr-list-title">
          Pull requests from this session
        </h2>
      ) : null}
      {groups.map((group) => (
        <div className="session-pr-group" key={group.name}>
          <h3>{group.name}</h3>
          {group.pullRequests.map((pullRequest) => (
            <a
              className="session-pr-row"
              href={pullRequestUrl(pullRequest, repo)}
              key={`${pullRequest.owner ?? repo}/${pullRequest.repo ?? ''}/${String(pullRequest.number)}`}
              target="_blank"
              rel="noreferrer"
            >
              <span
                className={`session-pr-ci-dot ${ciStateClass(pullRequest.ciState)}${pullRequest.state === 'Merged' ? ' is-merged' : ''}`}
                role="img"
                aria-label={ciStateLabel(pullRequest.ciState)}
                title={ciStateLabel(pullRequest.ciState)}
              />
              <span className="session-pr-row-copy">
                <span className="session-pr-row-title">
                  <span className="session-pr-number">
                    #{String(pullRequest.number)}
                  </span>{' '}
                  {pullRequest.title ?? 'Untitled pull request'}
                </span>
                <span className="session-pr-row-meta">
                  <span className="session-pr-row-repo">
                    {pullRequestRepo(pullRequest, repo)}
                  </span>
                  {isMobile ? (
                    <>
                      <span aria-hidden="true">·</span>
                      <span className="session-pr-row-mobile-state">
                        {pullRequest.state === 'Merged'
                          ? '✓ Merged'
                          : pullRequest.state}
                      </span>
                    </>
                  ) : null}
                </span>
              </span>
              {isMobile ? null : (
                <span
                  className={`session-pr-row-state${pullRequest.state === 'Merged' ? ' is-merged' : ''}`}
                >
                  {pullRequest.state === 'Merged'
                    ? '✓ Merged'
                    : pullRequest.state}
                </span>
              )}
            </a>
          ))}
        </div>
      ))}
      {isMobile ? (
        <div
          className={`session-pr-list-actions${mobileAction == null ? ' is-single' : ''}`}
        >
          <button
            className="session-pr-close"
            type="button"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Close
          </button>
          {mobileAction}
        </div>
      ) : null}
    </section>
  )

  return (
    <div
      className={`session-pr-menu${isMobile ? ' session-pr-menu--mobile' : ''}`}
    >
      <button
        className={`session-pr-trigger${isMobile ? ' session-pr-trigger--mobile' : ''}${open ? ' is-open' : ''}${allMerged ? ' is-all-merged' : ''}`}
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={triggerLabel}
        title={triggerLabel}
        onClick={() => {
          onOpenChange(!open)
        }}
      >
        {allMerged ? (
          <Check className="session-pr-check" aria-hidden="true" size={13} />
        ) : (
          <span
            className={`session-pr-ci-dot ${ciStateClass(currentPullRequest.ciState)}`}
            aria-hidden="true"
          />
        )}
        {allMerged ? (
          <span>Merged</span>
        ) : (
          <>
            <span className="session-pr-number">
              #{String(currentPullRequest.number)}
            </span>
            <span className="session-pr-summary-state">
              {currentPullRequest.state}
            </span>
          </>
        )}
        {additionalCount > 0 ? (
          <span className="session-pr-additional-count">
            +{String(additionalCount)}
          </span>
        ) : null}
        <ChevronDown
          className="session-pr-chevron"
          aria-hidden="true"
          size={13}
        />
      </button>
      {open ? (
        isMobile ? (
          <div
            className="session-pr-backdrop"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            {content}
          </div>
        ) : (
          <div className="session-pr-popover">{content}</div>
        )
      ) : null}
    </div>
  )
}
