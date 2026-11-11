import '#components/conversation-card.css'
import '#components/permission-card.css'

import { Button } from '@fohte/ui/button'

import type { ChatItem, SessionActions } from '#types'

export type PendingPermissionAction = {
  action: 'approve' | 'reject'
  status: 'sending' | 'sent'
  tool: string
  description: string
} | null

export type PermissionRequestIdentity = {
  tool: string
  description: string
}

type PermissionCardProps = {
  message: Extract<ChatItem, { type: 'permission' }>
  actions: SessionActions | null
  actionsReady: boolean
  pendingAction: PendingPermissionAction
  isCodex: boolean
  onAction: (
    action: 'permission/approve' | 'permission/reject',
    permission: PermissionRequestIdentity,
  ) => void
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
  const matchingPendingAction =
    pendingAction?.tool === message.tool &&
    pendingAction.description === message.description
      ? pendingAction
      : null

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
            className="aw-button--primary"
            disabled={
              !actionsReady || !canApprove || matchingPendingAction !== null
            }
            aria-busy={
              matchingPendingAction?.action === 'approve' &&
              matchingPendingAction.status === 'sending'
            }
            onClick={() => {
              onAction('permission/approve', message)
            }}
          >
            {matchingPendingAction?.action === 'approve' &&
            matchingPendingAction.status === 'sending'
              ? 'Approving…'
              : 'Approve'}
          </Button>
          <Button
            type="button"
            variant="plain"
            className="aw-button--outline"
            disabled={
              !actionsReady || !canReject || matchingPendingAction !== null
            }
            aria-busy={
              matchingPendingAction?.action === 'reject' &&
              matchingPendingAction.status === 'sending'
            }
            onClick={() => {
              onAction('permission/reject', message)
            }}
          >
            {matchingPendingAction?.action === 'reject' &&
            matchingPendingAction.status === 'sending'
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
      {matchingPendingAction?.status === 'sent' ? (
        <p className="permission-action-status" role="status">
          {matchingPendingAction.action === 'approve'
            ? 'Approval sent. Session is resuming…'
            : 'Rejection sent. Session is resuming…'}
        </p>
      ) : null}
    </article>
  )
}
