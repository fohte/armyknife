import { ConfirmDialog } from '#components/confirm-dialog'

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
    <ConfirmDialog
      title="Merge this pull request?"
      description={`Merge PR #${String(number)} using the confirmed head commit?`}
      confirmLabel="Confirm merge"
      pendingLabel="Merging…"
      pending={merging}
      className="pull-request-merge-confirm"
      portal={false}
      onCancel={onCancel}
      onConfirm={onConfirm}
    >
      <code className="pull-request-merge-head-sha">{headSha}</code>
    </ConfirmDialog>
  )
}
