import '#components/open-in-terminal-action.css'

import { Button } from '@fohte/ui/button'

import type { OpenInTerminalStatus } from '#types'

type OpenInTerminalActionProps = {
  status: OpenInTerminalStatus
  onClick: () => void
  announce?: boolean
}

export function OpenInTerminalAction({
  status,
  onClick,
  announce = true,
}: OpenInTerminalActionProps) {
  return (
    <div className="conversation-open-in-terminal">
      <Button
        type="button"
        variant="plain"
        disabled={status?.state === 'sending'}
        aria-busy={status?.state === 'sending'}
        onClick={onClick}
      >
        Open in terminal
      </Button>
      <p
        className="conversation-terminal-action-status"
        data-state={status?.state ?? 'idle'}
        role={announce ? 'status' : undefined}
        aria-live={announce ? 'polite' : undefined}
        aria-atomic={announce ? 'true' : undefined}
        aria-hidden={announce ? undefined : true}
      >
        {status?.state === 'sending'
          ? 'Switching tmux…'
          : status?.state === 'success'
            ? 'Tmux switched to this session.'
            : status?.state === 'error'
              ? `Could not switch tmux: ${status.message}`
              : ''}
      </p>
    </div>
  )
}
