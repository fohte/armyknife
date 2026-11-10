import { Button } from '@fohte/ui/button'

import type { ChatItem, SessionActions } from '#types'

export type PendingPermissionAction = {
  action: 'approve' | 'reject'
  status: 'sending' | 'sent'
} | null

type PermissionCardProps = {
  message: Extract<ChatItem, { type: 'permission' }>
  actions: SessionActions | null
  actionsReady: boolean
  pendingAction: PendingPermissionAction
  isCodex: boolean
  onAction: (action: 'permission/approve' | 'permission/reject') => void
  onOpenInTerminal: () => void
}

export function PermissionCard({
  message,
  actions,
  actionsReady,
  pendingAction,
  isCodex,
  onAction,
  onOpenInTerminal,
}: PermissionCardProps) {
  const canApprove = actions?.permission?.approve === true
  const canReject = actions?.permission?.reject === true
  const hasPermissionActions = canApprove || canReject

  return (
    <article className="chat-permission-card">
      <h2>Permission required</h2>
      <p className="permission-tool">{message.tool}</p>
      <p>{message.description}</p>
      {hasPermissionActions ? (
        <div className="permission-actions">
          <Button
            type="button"
            variant="plain"
            disabled={!actionsReady || !canApprove || pendingAction !== null}
            aria-busy={
              pendingAction?.action === 'approve' &&
              pendingAction.status === 'sending'
            }
            onClick={() => {
              onAction('permission/approve')
            }}
          >
            {pendingAction?.action === 'approve' &&
            pendingAction.status === 'sending'
              ? 'Approving…'
              : 'Approve'}
          </Button>
          <Button
            type="button"
            variant="plain"
            disabled={!actionsReady || !canReject || pendingAction !== null}
            aria-busy={
              pendingAction?.action === 'reject' &&
              pendingAction.status === 'sending'
            }
            onClick={() => {
              onAction('permission/reject')
            }}
          >
            {pendingAction?.action === 'reject' &&
            pendingAction.status === 'sending'
              ? 'Rejecting…'
              : 'Reject'}
          </Button>
        </div>
      ) : isCodex && actions !== null ? (
        <div className="conversation-terminal-fallback">
          <span>Review this permission in terminal.</span>
          {actions.openInTerminal ? (
            <Button type="button" variant="plain" onClick={onOpenInTerminal}>
              Open in terminal
            </Button>
          ) : null}
        </div>
      ) : null}
      {pendingAction?.status === 'sent' ? (
        <p className="permission-action-status" role="status">
          {pendingAction.action === 'approve'
            ? 'Approval sent. Session is resuming…'
            : 'Rejection sent. Session is resuming…'}
        </p>
      ) : null}
    </article>
  )
}
