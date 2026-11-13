import '#components/conversation-card.css'
import '#components/permission-card.css'

import { Button } from '@fohte/ui/button'

import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import type { ChatItem, OpenInTerminalStatus, SessionActions } from '#types'

type PendingPermissionAction = {
  action: 'approve' | 'reject' | 'retry'
  status: 'sending' | 'sent'
  requestId: string
}

export type PendingPermissionActions = Readonly<
  Record<string, PendingPermissionAction>
>

export type PendingPermissionActionState = PendingPermissionAction | null

export type PermissionRequestIdentity = {
  requestId: string
}

type PermissionCardProps = {
  message: Extract<ChatItem, { type: 'permission' }>
  actions: SessionActions | null
  actionsReady: boolean
  pendingAction: PendingPermissionActionState
  openInTerminalStatus?: OpenInTerminalStatus
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
  openInTerminalStatus = null,
  isCodex,
  onAction,
  onOpenInTerminal,
}: PermissionCardProps) {
  const canApprove =
    message.requestId !== undefined && actions?.permission?.approve === true
  const canReject =
    message.requestId !== undefined && actions?.permission?.reject === true
  const hasPermissionActions = canApprove || canReject
  const matchingPendingAction = pendingAction
  const permissionIdentity =
    message.requestId === undefined ? null : { requestId: message.requestId }

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
              if (permissionIdentity !== null) {
                onAction('permission/approve', permissionIdentity)
              }
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
              if (permissionIdentity !== null) {
                onAction('permission/reject', permissionIdentity)
              }
            }}
          >
            {matchingPendingAction?.action === 'reject' &&
            matchingPendingAction.status === 'sending'
              ? 'Rejecting…'
              : 'Reject'}
          </Button>
        </div>
      ) : (isCodex || message.requestId === undefined) && actions !== null ? (
        <div className="conversation-terminal-fallback">
          <span>Review this permission in terminal.</span>
          {actions.openInTerminal ? (
            <OpenInTerminalAction
              status={openInTerminalStatus}
              onClick={onOpenInTerminal}
              announce={false}
            />
          ) : null}
        </div>
      ) : null}
      {matchingPendingAction?.status === 'sent' ? (
        <p className="permission-action-status" role="status">
          {matchingPendingAction.action === 'approve'
            ? 'Approval sent. Waiting for Claude Code…'
            : 'Rejection sent. Waiting for Claude Code…'}
        </p>
      ) : null}
    </article>
  )
}
