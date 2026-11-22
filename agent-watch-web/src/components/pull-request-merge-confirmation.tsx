import { ConfirmDialog } from '#components/confirm-dialog'

type PullRequestMergeConfirmationProps = {
  number: number
  repository: string
  headSha: string
  merging: boolean
  mobile?: boolean
  onCancel: () => void
  onConfirm: () => void
}

export function PullRequestMergeConfirmation({
  number,
  repository,
  headSha,
  merging,
  mobile = false,
  onCancel,
  onConfirm,
}: PullRequestMergeConfirmationProps) {
  return (
    <ConfirmDialog
      title="Merge this pull request?"
      description={`Merge PR #${String(number)} in ${repository} using the confirmed head commit?`}
      confirmLabel="Confirm merge"
      pendingLabel="Merging…"
      pending={merging}
      mobile={mobile}
      className="pull-request-merge-confirm"
      portal={false}
      onCancel={onCancel}
      onConfirm={onConfirm}
    >
      <code className="pull-request-merge-head-sha">{headSha}</code>
    </ConfirmDialog>
  )
}
