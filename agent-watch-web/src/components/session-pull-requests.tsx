import '#components/session-pull-requests.css'

import { AlertCircle, GitPullRequest, LoaderCircle } from 'lucide-react'
import { useEffect, useRef } from 'react'

import {
  type AsyncActionStatus,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'
import { PullRequestMergeButton } from '#components/pull-request-merge-button'
import { useDismissableLayer } from '#components/use-dismissable-layer'
import type { PullRequest } from '#types'

type SessionPullRequestsProps = {
  pullRequests: readonly PullRequest[]
  repo: string
  isMobile: boolean
  open: boolean
  mergeStatus?: (pullRequest: PullRequest) => AsyncActionStatus | null
  canRetryMerge?: (pullRequest: PullRequest) => boolean
  mergeError?: (pullRequest: PullRequest) => string | undefined
  onMerge?: (pullRequest: PullRequest) => void
  onRetryMerge?: (pullRequest: PullRequest) => void
  onOpenChange: (open: boolean) => void
  presentation?: 'trigger' | 'list'
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

export function pullRequestMergeReason(pullRequest: PullRequest) {
  if (pullRequest.state !== 'Open') return pullRequest.state
  if (pullRequest.ciState !== 'Successful') {
    return ciStateLabel(pullRequest.ciState)
  }
  return 'Head SHA unavailable'
}

export function sessionPullRequestStatusLabel(
  pullRequests: readonly PullRequest[],
  mergeStatus: (pullRequest: PullRequest) => AsyncActionStatus | null,
) {
  const summary = pullRequestStatusSummary(pullRequests, mergeStatus)
  if (summary.currentPullRequest === undefined) return null

  return `PR #${String(summary.currentPullRequest.number)}, ${summary.currentPullRequestMerged ? 'merged' : summary.currentPullRequest.state.toLowerCase()}, ${ciStateLabel(summary.currentPullRequest.ciState)}${summary.hasMergeInProgress ? ', a pull request is merging' : ''}${summary.hasMergeFailure ? ', a merge failed' : ''}${summary.additionalCount > 0 ? `. ${String(summary.additionalCount)} more PR${summary.additionalCount === 1 ? '' : 's'} from this session` : ''}`
}

export function SessionPullRequestStatusBadge({
  pullRequests,
  mergeStatus,
}: {
  pullRequests: readonly PullRequest[]
  mergeStatus: (pullRequest: PullRequest) => AsyncActionStatus | null
}) {
  const summary = pullRequestStatusSummary(pullRequests, mergeStatus)
  if (summary.currentPullRequest === undefined) return null

  return (
    <span className="session-pr-status-badge" aria-hidden="true">
      {summary.hasMergeFailure ? (
        <span className="session-pr-status-mark is-failed">
          <AlertCircle aria-hidden="true" size={15} />
        </span>
      ) : summary.hasMergeInProgress ? (
        <LoaderCircle className="session-pr-status-mark is-merging" size={14} />
      ) : summary.currentPullRequestMerged ? null : (
        <span
          className={`session-pr-status-ci-dot ${ciStateClass(summary.currentPullRequest.ciState)}`}
        />
      )}
      <span className="session-pr-status-count">
        {summary.pullRequestCount}
      </span>
    </span>
  )
}

function pullRequestRowMergeReason(pullRequest: PullRequest) {
  if (pullRequest.state === 'Open' && pullRequest.ciState === 'Failed') {
    return 'CI failing'
  }
  return pullRequestMergeReason(pullRequest)
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

function getRepresentativePullRequest(pullRequests: readonly PullRequest[]) {
  return (
    pullRequests.find((pullRequest) => pullRequest.isCurrentBranch === true) ??
    pullRequests.find((pullRequest) => pullRequest.state === 'Open') ??
    pullRequests[0]
  )
}

function pullRequestStatusSummary(
  pullRequests: readonly PullRequest[],
  mergeStatus: (pullRequest: PullRequest) => AsyncActionStatus | null,
) {
  const currentPullRequest = getRepresentativePullRequest(pullRequests)
  const currentMergeStatus =
    currentPullRequest === undefined ? null : mergeStatus(currentPullRequest)
  const hasMergeFailure = pullRequests.some(
    (pullRequest) =>
      pullRequest.state === 'Open' &&
      mergeStatus(pullRequest) === 'mergeFailed',
  )
  const hasMergeInProgress = pullRequests.some(
    (pullRequest) =>
      pullRequest.state === 'Open' &&
      ['merging', 'queued'].includes(mergeStatus(pullRequest) ?? ''),
  )

  return {
    currentPullRequest,
    currentPullRequestMerged:
      currentPullRequest?.state === 'Merged' ||
      ['merged', 'alreadyMerged'].includes(currentMergeStatus ?? ''),
    hasMergeFailure,
    hasMergeInProgress,
    allMerged: pullRequests.every(
      (pullRequest) =>
        pullRequest.state === 'Merged' ||
        ['merged', 'alreadyMerged'].includes(mergeStatus(pullRequest) ?? ''),
    ),
    pullRequestCount: pullRequests.length,
    additionalCount: Math.max(pullRequests.length - 1, 0),
  }
}

export function SessionPullRequests({
  pullRequests,
  repo,
  isMobile,
  open,
  mergeStatus,
  canRetryMerge,
  mergeError,
  onMerge,
  onRetryMerge,
  onOpenChange,
  presentation = 'trigger',
}: SessionPullRequestsProps) {
  const triggerRef = useRef<HTMLButtonElement>(null)
  const contentRef = useRef<HTMLElement>(null)
  const actionFeedback = useOptionalAsyncActionFeedback()
  const mergeStatusFor = (pullRequest: PullRequest) =>
    pullRequest.state === 'Merged'
      ? 'merged'
      : pullRequest.state === 'Open'
        ? (mergeStatus?.(pullRequest) ?? null)
        : null
  const statusSummary = pullRequestStatusSummary(pullRequests, mergeStatusFor)
  const currentPullRequest = statusSummary.currentPullRequest
  const hasPullRequests = currentPullRequest !== undefined
  const triggerLabel =
    currentPullRequest === undefined
      ? ''
      : `${statusSummary.allMerged ? `All ${String(statusSummary.pullRequestCount)} pull requests are merged. ` : ''}PR #${String(currentPullRequest.number)}: ${currentPullRequest.title ?? 'Untitled pull request'}, ${statusSummary.currentPullRequestMerged ? 'merged' : currentPullRequest.state.toLowerCase()}, ${ciStateLabel(currentPullRequest.ciState)}${statusSummary.hasMergeInProgress ? ', a pull request is merging' : ''}${statusSummary.hasMergeFailure ? ', a merge failed' : ''}${statusSummary.additionalCount > 0 ? `. ${String(statusSummary.additionalCount)} more PR${statusSummary.additionalCount === 1 ? '' : 's'} from this session` : ''}`

  useDismissableLayer(
    open,
    onOpenChange,
    isMobile && actionFeedback !== null
      ? [triggerRef, contentRef, actionFeedback.toastRef]
      : [triggerRef, contentRef],
  )

  useEffect(() => {
    const setToastBottomOffset = actionFeedback?.setToastBottomOffset
    if (
      !isMobile ||
      !open ||
      !hasPullRequests ||
      setToastBottomOffset === undefined
    ) {
      setToastBottomOffset?.(null)
      return
    }

    const updateOffset = () => {
      const content = contentRef.current
      if (content === null) return
      setToastBottomOffset(
        Math.max(
          112,
          window.innerHeight - content.getBoundingClientRect().top + 8,
        ),
      )
    }

    updateOffset()
    const observer = new ResizeObserver(updateOffset)
    const content = contentRef.current
    if (content !== null) observer.observe(content)
    window.addEventListener('resize', updateOffset)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', updateOffset)
      setToastBottomOffset(null)
    }
  }, [actionFeedback?.setToastBottomOffset, hasPullRequests, isMobile, open])

  if (!hasPullRequests) return null

  const groups = pullRequestGroups(pullRequests)
  const listTitle =
    presentation === 'list'
      ? 'Pull requests'
      : 'Pull requests from this session'
  const content = (
    <section
      ref={contentRef}
      className={`session-pr-list${isMobile ? ' session-pr-list--mobile' : ''}${presentation === 'list' ? ' session-pr-list--panel' : ''}`}
      role={presentation === 'list' ? undefined : 'dialog'}
      aria-label={listTitle}
      aria-modal={presentation === 'list' ? undefined : isMobile || undefined}
      tabIndex={presentation === 'list' ? undefined : -1}
    >
      {isMobile ? <h2 className="session-pr-list-title">{listTitle}</h2> : null}
      {groups.map((group) => (
        <div className="session-pr-group" key={group.name}>
          <h3>{group.name}</h3>
          {group.pullRequests.map((pullRequest) => {
            const status = mergeStatusFor(pullRequest)
            const canRetry = canRetryMerge?.(pullRequest) ?? true
            const entryStatus = mergeEntryStatus(pullRequest, status, canRetry)
            const error = mergeError?.(pullRequest)
            return (
              <div
                className="session-pr-row"
                key={`${pullRequest.owner ?? repo}/${pullRequest.repo ?? ''}/${String(pullRequest.number)}`}
              >
                <span
                  className={`session-pr-ci-dot ${ciStateClass(pullRequest.ciState)}${pullRequest.state === 'Merged' || ['merged', 'alreadyMerged'].includes(entryStatus) ? ' is-merged' : ''}`}
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
                    {isMobile ? (
                      <span className="session-pr-row-mobile-state">
                        · {mobilePullRequestState(pullRequest, status)}
                      </span>
                    ) : null}
                  </span>
                </a>
                {isMobile &&
                pullRequest.state !== 'Open' &&
                !['merged', 'alreadyMerged'].includes(entryStatus) ? null : (
                  <PullRequestMergeButton
                    status={entryStatus}
                    pullRequestNumber={pullRequest.number}
                    isMobile={isMobile}
                    placement="row"
                    {...(pullRequest.owner === undefined ||
                    pullRequest.repo === undefined
                      ? {}
                      : {
                          pullRequestRepo: `${pullRequest.owner}/${pullRequest.repo}`,
                        })}
                    unavailableReason={pullRequestRowMergeReason(pullRequest)}
                    canRetry={canRetry}
                    onMerge={() => {
                      onMerge?.(pullRequest)
                    }}
                    onRetry={() => {
                      onRetryMerge?.(pullRequest)
                    }}
                    {...(error === undefined ? {} : { title: error })}
                  />
                )}
              </div>
            )
          })}
        </div>
      ))}
      {isMobile && presentation === 'trigger' ? (
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

  if (presentation === 'list') {
    return content
  }

  return (
    <div
      className={`session-pr-menu${isMobile ? ' session-pr-menu--mobile' : ''}`}
    >
      <button
        ref={triggerRef}
        className={`session-pr-trigger${isMobile ? ' session-pr-trigger--mobile' : ''}${open ? ' is-open' : ''}${statusSummary.allMerged ? ' is-all-merged' : ''}${statusSummary.hasMergeFailure ? ' is-merge-failed' : ''}${statusSummary.hasMergeInProgress ? ' is-merge-in-progress' : ''}${!isMobile && (statusSummary.hasMergeFailure || statusSummary.hasMergeInProgress) ? ' is-merge-mark-inline' : ''}`}
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
        {statusSummary.hasMergeFailure ? (
          <span
            className={`session-pr-merge-mark${isMobile ? ' is-mobile' : ' is-inline'} is-failed`}
            aria-hidden="true"
          >
            <AlertCircle size={15} />
          </span>
        ) : statusSummary.hasMergeInProgress ? (
          <span
            className={`session-pr-merge-mark${isMobile ? ' is-mobile' : ' is-inline'} is-merging`}
            aria-hidden="true"
          >
            <LoaderCircle size={isMobile ? 15 : 13} />
          </span>
        ) : statusSummary.currentPullRequestMerged ? null : (
          <span
            className={`session-pr-trigger-ci-dot ${ciStateClass(currentPullRequest.ciState)}`}
            aria-hidden="true"
          />
        )}
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

function mergeEntryStatus(
  pullRequest: PullRequest,
  status: AsyncActionStatus | null | undefined,
  canRetry: boolean,
) {
  if (pullRequest.state === 'Merged' || status === 'merged') return 'merged'
  if (status === 'alreadyMerged') return 'alreadyMerged'
  if (status === 'mergeFailed') return canRetry ? 'mergeFailed' : 'unavailable'
  if (status === 'merging') return 'merging'
  if (status === 'queued') return 'queued'
  return isPullRequestMergeable(pullRequest) ? 'ready' : 'unavailable'
}

function mobilePullRequestState(
  pullRequest: PullRequest,
  status: AsyncActionStatus | null,
) {
  if (pullRequest.state === 'Merged' || status === 'merged') return 'Merged'
  if (status === 'alreadyMerged') return 'Already merged'
  return pullRequest.state
}
