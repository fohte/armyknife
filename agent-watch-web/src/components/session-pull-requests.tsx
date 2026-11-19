import '#components/session-pull-requests.css'

import { GitPullRequest } from 'lucide-react'
import { useRef } from 'react'

import { PullRequestMergeAction } from '#components/pull-request-merge-action'
import { useDismissableLayer } from '#components/use-dismissable-layer'
import type { PullRequest } from '#types'

type SessionPullRequestsProps = {
  pullRequests: readonly PullRequest[]
  repo: string
  isMobile: boolean
  open: boolean
  onMerge: (pullRequest: PullRequest) => void
  isMerging: (pullRequest: PullRequest) => boolean
  isMergeSuccessful: (pullRequest: PullRequest) => boolean
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

export function isPullRequestMergeable(
  pullRequest: PullRequest,
): pullRequest is PullRequest & {
  state: 'Open'
  ciState: 'Successful'
  headSha: string
} {
  return (
    pullRequest.state === 'Open' &&
    pullRequest.ciState === 'Successful' &&
    pullRequest.headSha !== null &&
    pullRequest.headSha !== undefined
  )
}

function pullRequestMergeReason(pullRequest: PullRequest) {
  if (pullRequest.state !== 'Open') return pullRequest.state
  if (pullRequest.ciState !== 'Successful') {
    return ciStateLabel(pullRequest.ciState)
  }
  return 'Head SHA unavailable'
}

function pullRequestGroups(pullRequests: readonly PullRequest[]) {
  const groups: PullRequestGroup[] = [
    {
      name: 'This branch',
      pullRequests: pullRequests.filter(
        (pullRequest) =>
          pullRequest.isCurrentBranch === true &&
          pullRequest.state !== 'Merged',
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

export function getRepresentativePullRequest(
  pullRequests: readonly PullRequest[],
) {
  return (
    pullRequests.find((pullRequest) => pullRequest.isCurrentBranch === true) ??
    pullRequests.find((pullRequest) => pullRequest.state === 'Open') ??
    pullRequests[0]
  )
}

export function SessionPullRequests({
  pullRequests,
  repo,
  isMobile,
  open,
  onMerge,
  isMerging,
  isMergeSuccessful,
  onOpenChange,
}: SessionPullRequestsProps) {
  const triggerRef = useRef<HTMLButtonElement>(null)
  const contentRef = useRef<HTMLElement>(null)
  const currentPullRequest = getRepresentativePullRequest(pullRequests)
  const allMerged = pullRequests.every(
    (pullRequest) => pullRequest.state === 'Merged',
  )
  const additionalCount = Math.max(pullRequests.length - 1, 0)
  const triggerLabel =
    currentPullRequest === undefined
      ? ''
      : `${allMerged ? `All ${String(pullRequests.length)} pull requests are merged. ` : ''}PR #${String(currentPullRequest.number)}: ${currentPullRequest.title ?? 'Untitled pull request'}, ${currentPullRequest.state.toLowerCase()}, ${ciStateLabel(currentPullRequest.ciState)}${additionalCount > 0 ? `. ${String(additionalCount)} more PR${additionalCount === 1 ? '' : 's'} from this session` : ''}`

  useDismissableLayer(open, onOpenChange, [triggerRef, contentRef])

  if (pullRequests.length === 0 || currentPullRequest === undefined) return null

  const groups = pullRequestGroups(pullRequests)
  const content = (
    <section
      ref={contentRef}
      className={`session-pr-list${isMobile ? ' session-pr-list--mobile' : ''}`}
      role="dialog"
      aria-label="Pull requests from this session"
      aria-modal={isMobile || undefined}
      tabIndex={-1}
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
            <div
              className="session-pr-row"
              key={`${pullRequest.owner ?? repo}/${pullRequest.repo ?? ''}/${String(pullRequest.number)}`}
            >
              <span
                className={`session-pr-ci-dot ${ciStateClass(pullRequest.ciState)}${pullRequest.state === 'Merged' ? ' is-merged' : ''}`}
                role="img"
                aria-label={ciStateLabel(pullRequest.ciState)}
                title={ciStateLabel(pullRequest.ciState)}
              />
              <a
                className="session-pr-row-copy"
                href={pullRequestUrl(pullRequest, repo)}
                target="_blank"
                rel="noreferrer"
              >
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
                </span>
              </a>
              {isPullRequestMergeable(pullRequest) ? (
                isMergeSuccessful(pullRequest) ? (
                  <span className="session-pr-row-state is-merged">
                    ✓ Merged
                  </span>
                ) : (
                  <PullRequestMergeAction
                    ariaLabel={`Merge PR #${String(pullRequest.number)}`}
                    merging={isMerging(pullRequest)}
                    onClick={() => {
                      onMerge(pullRequest)
                    }}
                  />
                )
              ) : (
                <span
                  className={`session-pr-row-state${pullRequest.state === 'Open' ? ` ${ciStateClass(pullRequest.ciState)}` : ''}${pullRequest.state === 'Merged' ? ' is-merged' : ''}`}
                >
                  {pullRequest.state === 'Merged'
                    ? '✓ Merged'
                    : pullRequestMergeReason(pullRequest)}
                </span>
              )}
            </div>
          ))}
        </div>
      ))}
      {isMobile ? (
        <div className="session-pr-list-actions is-single">
          <button
            className="session-pr-close"
            type="button"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Close
          </button>
        </div>
      ) : null}
    </section>
  )

  return (
    <div
      className={`session-pr-menu${isMobile ? ' session-pr-menu--mobile' : ''}`}
    >
      <button
        ref={triggerRef}
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
        <GitPullRequest
          className="session-pr-icon"
          aria-hidden="true"
          size={isMobile ? 18 : 14}
          strokeWidth={1.7}
        />
        <span
          className={`session-pr-trigger-ci-dot ${ciStateClass(currentPullRequest.ciState)}`}
          aria-hidden="true"
        />
        <span className="session-pr-count" aria-hidden="true">
          {pullRequests.length}
        </span>
      </button>
      {open ? (
        isMobile ? (
          <div className="session-pr-backdrop">{content}</div>
        ) : (
          <div className="session-pr-popover">{content}</div>
        )
      ) : null}
    </div>
  )
}
