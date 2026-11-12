import '#components/open-in-terminal-action.css'

import { Button } from '@fohte/ui/button'
import { SquareTerminal } from 'lucide-react'

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
        className="conversation-open-in-terminal-button"
        type="button"
        variant="plain"
        aria-label="Open in terminal"
        title="Open in terminal"
        disabled={status?.state === 'sending'}
        aria-busy={status?.state === 'sending'}
        onClick={onClick}
      >
        <SquareTerminal aria-hidden="true" size={16} />
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
