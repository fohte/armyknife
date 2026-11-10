import { Button } from '@fohte/ui/button'

type CopySessionIdButtonProps = {
  status: 'idle' | 'copied' | 'failed'
  compact?: boolean
  onCopy: () => void
}

export function CopySessionIdButton({
  status,
  compact = false,
  onCopy,
}: CopySessionIdButtonProps) {
  const label =
    status === 'copied'
      ? 'Copied'
      : status === 'failed'
        ? 'Copy failed'
        : compact
          ? 'Copy ID'
          : 'Copy session ID'
  const accessibleStatus =
    status === 'copied'
      ? 'Session ID copied'
      : status === 'failed'
        ? 'Could not copy session ID'
        : ''

  return (
    <>
      <Button
        className="conversation-copy-session-id"
        type="button"
        variant="plain"
        aria-label={status === 'idle' ? 'Copy session ID' : accessibleStatus}
        title="Copy session ID"
        onClick={onCopy}
      >
        {label}
      </Button>
      {accessibleStatus === '' ? null : (
        <span className="sr-only" role="status">
          {accessibleStatus}
        </span>
      )}
    </>
  )
}
