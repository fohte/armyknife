import '#components/copy-session-id-button.css'

import { Button } from '@fohte/ui/button'
import { Check, Copy } from 'lucide-react'

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
  const icon = status === 'copied' ? <Check /> : <Copy />

  return (
    <>
      <Button
        className={
          compact
            ? 'conversation-copy-session-id'
            : 'conversation-copy-session-id aw-button--outline'
        }
        data-compact={compact ? 'true' : undefined}
        type="button"
        variant="plain"
        aria-label={status === 'idle' ? 'Copy session ID' : accessibleStatus}
        title="Copy session ID"
        onClick={onCopy}
      >
        {compact ? null : <span aria-hidden="true">{icon}</span>}
        <span className={compact ? undefined : 'sr-only'}>{label}</span>
      </Button>
      {accessibleStatus === '' ? null : (
        <span className="sr-only" role="status">
          {accessibleStatus}
        </span>
      )}
    </>
  )
}
