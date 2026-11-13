import '#components/pull-request-list-panel.css'

type PullRequestMergeConfirmationProps = {
  number: number
  headSha: string
  merging: boolean
  onCancel: () => void
  onConfirm: () => void
}

export function PullRequestMergeConfirmation({
  number,
  headSha,
  merging,
  onCancel,
  onConfirm,
}: PullRequestMergeConfirmationProps) {
  return (
    <div className="pull-request-list-confirm-backdrop">
      <section
        className="pull-request-list-confirm"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="pull-request-merge-title"
      >
        <h3 id="pull-request-merge-title">Merge this pull request?</h3>
        <p>Merge PR #{String(number)} using the confirmed head commit?</p>
        <code>{headSha}</code>
        <div className="pull-request-list-confirm-actions">
          <button type="button" disabled={merging} onClick={onCancel}>
            Cancel
          </button>
          <button type="button" disabled={merging} onClick={onConfirm}>
            {merging ? 'Merging…' : 'Confirm merge'}
          </button>
        </div>
      </section>
    </div>
  )
}
