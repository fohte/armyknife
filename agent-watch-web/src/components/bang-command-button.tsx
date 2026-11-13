import '#components/bang-command.css'

export type CommandStatus = 'idle' | 'running' | 'succeeded' | 'failed'

type BangCommandButtonProps = {
  label: string
  status: CommandStatus
  message?: string
  busy?: boolean
  onClick: () => void
}

export function BangCommandButton({
  label,
  status,
  message,
  busy = false,
  onClick,
}: BangCommandButtonProps) {
  return (
    <span className="bang-command-control">
      <button
        aria-label={label}
        className="bang-command-button"
        disabled={busy || status === 'running'}
        onClick={onClick}
        type="button"
      >
        {status === 'running' ? 'Running…' : label}
      </button>
      {message !== undefined && (
        <span
          aria-live="polite"
          className="bang-command-status"
          data-state={status}
          role="status"
        >
          {message}
        </span>
      )}
    </span>
  )
}
