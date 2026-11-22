import '#components/merge-band.css'

import { Button } from '@fohte/ui/button'
import { LoaderCircle } from 'lucide-react'

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
          {status === 'ready' ? (
            <Button
              className="merge-band-action is-ready"
              type="button"
              variant="secondary"
              aria-label={`Merge PR #${String(pullRequest.number)}`}
              onClick={() => {
                onMerge(pullRequest)
              }}
            >
              Merge
            </Button>
          ) : status === 'mergeFailed' && canRetry !== false ? (
            <Button
              className="merge-band-action is-failed"
              type="button"
              variant="outline"
              aria-label={`Retry merging PR #${String(pullRequest.number)}`}
              onClick={() => {
                onRetry(pullRequest)
              }}
            >
              Retry
            </Button>
          ) : status === 'mergeFailed' ? null : (
            <Button
              className="merge-band-action is-pending"
              type="button"
              variant="outline"
              disabled
              aria-label={`${statusLabels[status]} PR #${String(pullRequest.number)}`}
            >
              <LoaderCircle aria-hidden="true" size={13} />
              {status === 'queued' ? 'Queued' : 'Merging…'}
            </Button>
          )}
        </div>
      ))}
    </section>
  )
}
