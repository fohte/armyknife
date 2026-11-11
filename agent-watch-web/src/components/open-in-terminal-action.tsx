import '#components/open-in-terminal-action.css'

import { Button } from '@fohte/ui/button'

import type { OpenInTerminalStatus } from '#types'

type OpenInTerminalActionProps = {
  status: OpenInTerminalStatus
  onClick: () => void
}

export function OpenInTerminalAction({
  status,
  onClick,
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
      {status === null ? null : (
        <p
          className="conversation-terminal-action-status"
          data-state={status.state}
          role={status.state === 'error' ? 'alert' : 'status'}
        >
          {status.state === 'sending'
            ? 'Switching tmux…'
            : status.state === 'success'
              ? 'Tmux switched to this session.'
              : `Could not switch tmux: ${status.message}`}
        </p>
      )}
    </div>
  )
}
