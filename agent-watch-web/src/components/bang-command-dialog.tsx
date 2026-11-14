import '#components/bang-command.css'

import { createPortal } from 'react-dom'

type BangCommandDialogProps = {
  open: boolean
  commands: readonly string[]
  cwd: string
  running?: boolean
  onCancel: () => void
  onConfirm: () => void
}

export function BangCommandDialog({
  open,
  commands,
  cwd,
  running = false,
  onCancel,
  onConfirm,
}: BangCommandDialogProps) {
  if (!open) return null

  const dialog = (
    <div
      aria-labelledby="bang-command-dialog-title"
      className="bang-command-dialog-backdrop"
    >
      <section aria-modal="true" className="bang-command-dialog" role="dialog">
        <h2 id="bang-command-dialog-title">Run agent command?</h2>
        <p>Review the command and working directory before it runs.</p>
        <dl>
          <dt>Working directory</dt>
          <dd>
            <code>{cwd}</code>
          </dd>
          <dt>Command</dt>
          <dd>
            <pre>{commands.join('\n')}</pre>
          </dd>
        </dl>
        {running ? (
          <p aria-live="polite" role="status">
            Running command…
          </p>
        ) : (
          <div className="bang-command-dialog-actions">
            <button onClick={onCancel} type="button">
              Cancel
            </button>
            <button onClick={onConfirm} type="button">
              Run command
            </button>
          </div>
        )}
      </section>
    </div>
  )
  return typeof document === 'undefined'
    ? dialog
    : createPortal(dialog, document.body)
}
