import { Button } from '@fohte/ui/button'

type PullRequestMergeActionProps = {
  ariaLabel?: string
  merging: boolean
  onClick: () => void
}

export function PullRequestMergeAction({
  ariaLabel,
  merging,
  onClick,
}: PullRequestMergeActionProps) {
  return (
    <Button
      className="conversation-pr-merge-button"
      type="button"
      variant="outline"
      disabled={merging}
      aria-label={ariaLabel}
      onClick={onClick}
    >
      {merging ? 'Merging…' : 'Merge'}
    </Button>
  )
}
