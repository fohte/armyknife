import { Button } from '@fohte/ui/button'

import type { ChatItem, Session, SessionActions } from '#types'

type PermissionCardProps = {
  message: Extract<ChatItem, { type: 'permission' }>
  actions: SessionActions | null
  actionsReady: boolean
  engine: Session['engine']
  onAction: (action: string) => Promise<boolean>
}

export function PermissionCard({
  message,
  actions,
  actionsReady,
  engine,
  onAction,
}: PermissionCardProps) {
  const permission = actions?.permission

  return (
    <article className="chat-permission-card">
      <h2>Permission required</h2>
      <p className="permission-tool">{message.tool}</p>
      <p>{message.description}</p>
      {permission != null && (permission.approve || permission.reject) ? (
        <div className="permission-actions">
          <Button
            type="button"
            variant="plain"
            disabled={!actionsReady || !permission.approve}
            onClick={() => {
              void onAction('permission/approve')
            }}
          >
            Approve
          </Button>
          <Button
            type="button"
            variant="plain"
            disabled={!actionsReady || !permission.reject}
            onClick={() => {
              void onAction('permission/reject')
            }}
          >
            Reject
          </Button>
        </div>
      ) : engine === 'Codex' && actions !== null ? (
        <div className="conversation-terminal-fallback">
          <span>Review this permission in terminal.</span>
          {actions.openInTerminal ? (
            <Button
              type="button"
              variant="plain"
              onClick={() => {
                void onAction('focus')
              }}
            >
              Open in terminal
            </Button>
          ) : null}
        </div>
      ) : null}
    </article>
  )
}
