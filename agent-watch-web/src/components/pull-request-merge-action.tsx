import { Button } from '@fohte/ui/button'

type PullRequestMergeActionProps = {
  merging: boolean
  onClick: () => void
}

export function PullRequestMergeAction({
  merging,
  onClick,
}: PullRequestMergeActionProps) {
  return (
    <Button
      className="conversation-pr-merge-button"
      type="button"
      variant="plain"
      disabled={merging}
      onClick={onClick}
    >
      {merging ? 'Merging…' : 'Merge'}
    </Button>
  )
}
