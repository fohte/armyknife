import '#components/permission-denied-card.css'

import { Button } from '@fohte/ui/button'

import type { PendingPermissionActionState } from '#components/permission-card'
import type { ChatItem } from '#types'

type PermissionDeniedCardProps = {
  message: Extract<ChatItem, { type: 'permission_denied' }>
  canRetry: boolean
  actionsReady: boolean
  pendingAction: PendingPermissionActionState
  onRetry: () => void
}

export function PermissionDeniedCard({
  message,
  canRetry,
  actionsReady,
  pendingAction,
  onRetry,
}: PermissionDeniedCardProps) {
  const retryAction = pendingAction?.action === 'retry' ? pendingAction : null
  return (
    <article className="chat-permission-denied-card">
      <h2>{message.resolved ? 'Retry approved' : 'Operation denied'}</h2>
      <p className="permission-denied-tool">{message.tool}</p>
      <pre className="permission-denied-input">{message.inputSummary}</pre>
      {message.reason === null ? null : (
        <p className="permission-denied-reason">{message.reason}</p>
      )}
      {!message.retryable ? (
        <p className="permission-denied-unavailable" role="status">
          This denial has no classifier verdict and cannot be approved here.
        </p>
      ) : message.resolved ? (
        <p className="permission-denied-complete" role="status">
          The one-time retry instruction was sent.
        </p>
      ) : canRetry ? (
        <div className="permission-denied-actions">
          <Button
            type="button"
            variant="plain"
            className="aw-button--primary"
            disabled={!actionsReady || pendingAction !== null}
            aria-busy={retryAction?.status === 'sending'}
            onClick={() => {
              onRetry()
            }}
          >
            {retryAction?.status === 'sending' ? 'Sending…' : 'Allow and retry'}
          </Button>
        </div>
      ) : null}
      {!message.resolved && retryAction?.status === 'sent' ? (
        <p className="permission-denied-sent" role="status">
          Retry instruction sent.
        </p>
      ) : null}
    </article>
  )
}
