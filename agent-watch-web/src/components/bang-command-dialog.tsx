import '#components/bang-command.css'

import { ConfirmDialog } from '#components/confirm-dialog'

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

  return (
    <ConfirmDialog
      title="Run agent command?"
      description="Review the command and working directory before it runs."
      confirmLabel="Run command"
      pendingLabel="Running command…"
      pending={running}
      className="bang-command-dialog"
      dialogRole="dialog"
      onCancel={onCancel}
      onConfirm={onConfirm}
    >
      <dl className="bang-command-dialog-details">
        <dt>Working directory</dt>
        <dd>
          <code>{cwd}</code>
        </dd>
        <dt>Command</dt>
        <dd>
          <pre>{commands.join('\n')}</pre>
        </dd>
      </dl>
    </ConfirmDialog>
  )
}
