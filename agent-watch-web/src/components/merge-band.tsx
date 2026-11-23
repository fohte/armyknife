import '#components/merge-band.css'

import { PullRequestMergeButton } from '#components/pull-request-merge-button'
import type { PullRequest } from '#types'

type MergeBandRow = {
  pullRequest: PullRequest
  status: 'ready' | 'merging' | 'queued' | 'mergeFailed'
  error?: string
  canRetry?: boolean
}

type MergeBandProps = {
  rows: readonly MergeBandRow[]
  isMobile: boolean
  onMerge: (pullRequest: PullRequest) => void
  onRetry: (pullRequest: PullRequest) => void
}

const statusLabels: Record<MergeBandRow['status'], string> = {
  ready: 'Ready to merge',
  merging: 'Merging…',
  queued: 'In the merge queue',
  mergeFailed: 'Merge failed',
}

export function MergeBand({
  rows,
  isMobile,
  onMerge,
  onRetry,
}: MergeBandProps) {
  if (rows.length === 0) return null

  return (
    <section
      className={`merge-band${isMobile ? ' is-mobile' : ''}${rows.some(({ status }) => status === 'mergeFailed') ? ' is-failed' : ''}`}
      aria-label="Pull requests ready to merge"
    >
      {rows.map(({ pullRequest, status, error, canRetry }) => (
        <div
          className={`merge-band-row is-${status}`}
          key={`${pullRequest.owner ?? ''}/${pullRequest.repo ?? ''}/${String(pullRequest.number)}`}
          role="status"
        >
          <div className="merge-band-copy">
            <div className="merge-band-heading">
              <span className="merge-band-dot" aria-hidden="true" />
              <strong>{statusLabels[status]}</strong>
            </div>
            <p className="merge-band-detail">
              <span className="merge-band-number">
                #{String(pullRequest.number)}
              </span>{' '}
              {status === 'mergeFailed'
                ? (error ?? 'Merge could not be completed. Retry to try again.')
                : (pullRequest.title ?? 'Untitled pull request')}
            </p>
          </div>
          <PullRequestMergeButton
            status={status}
            pullRequestNumber={pullRequest.number}
            isMobile={isMobile}
            placement="band"
            {...(pullRequest.owner === undefined ||
            pullRequest.repo === undefined
              ? {}
              : {
                  pullRequestRepo: `${pullRequest.owner}/${pullRequest.repo}`,
                })}
            {...(canRetry === undefined ? {} : { canRetry })}
            onMerge={() => {
              onMerge(pullRequest)
            }}
            onRetry={() => {
              onRetry(pullRequest)
            }}
          />
        </div>
      ))}
    </section>
  )
}
