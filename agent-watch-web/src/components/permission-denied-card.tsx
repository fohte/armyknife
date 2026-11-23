import '#components/conversation-card.css'
import '#components/permission-card.css'
import '#components/permission-denied-card.css'

import { Button } from '@fohte/ui/button'
import { useState } from 'react'

import type { PendingPermissionActionState } from '#components/permission-card'
import { formatPermissionCommand } from '#components/permission-command'
import { SyntaxCodeBlock } from '#components/syntax-code-block'
import type { ChatItem } from '#types'

type PermissionDeniedCardProps = {
  message: Extract<ChatItem, { type: 'permission_denied' }>
  canRetry: boolean
  actionsReady: boolean
  pendingAction: PendingPermissionActionState
  canDismiss: boolean
  onRetry: () => void
  onDismiss: () => void
  defaultOpen?: boolean
}

export function PermissionDeniedCard({
  message,
  canRetry,
  actionsReady,
  pendingAction,
  canDismiss,
  onRetry,
  onDismiss,
  defaultOpen = false,
}: PermissionDeniedCardProps) {
  const [expanded, setExpanded] = useState(defaultOpen)
  const retryAction = pendingAction?.action === 'retry' ? pendingAction : null
  const approved = message.resolution === 'approved' || message.resolved
  if (message.resolution === 'dismissed') {
    return (
      <article className="chat-permission-card chat-permission-denied-card chat-permission-denied-card--dismissed">
        <details
          open={expanded}
          onToggle={(event) => {
            setExpanded(event.currentTarget.open)
          }}
        >
          <summary>
            <span>Operation denied · Dismissed </span>
            <code>{message.command ?? message.tool}</code>{' '}
            <span className="permission-denied-chevron" aria-hidden="true">
              ▸
            </span>
          </summary>
          <div className="permission-denied-details">
            <h2>Operation denied</h2>
            <PermissionDeniedDetails message={message} />
          </div>
        </details>
      </article>
    )
  }
  return (
    <article className="chat-permission-card chat-permission-denied-card">
      <h2>{approved ? 'Retry approved' : 'Operation denied'}</h2>
      <PermissionDeniedDetails message={message} />
      {!message.retryable ? (
        <p className="permission-denied-unavailable" role="status">
          This denial has no classifier verdict and cannot be approved here.
        </p>
      ) : approved ? (
        <p className="permission-denied-complete" role="status">
          The one-time retry instruction was sent.
        </p>
      ) : null}
      {!message.resolved ? (
        <div className="permission-actions">
          {message.retryable && canRetry ? (
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
              {retryAction?.status === 'sending'
                ? 'Sending…'
                : 'Allow and retry'}
            </Button>
          ) : null}
          <DismissButton
            canDismiss={canDismiss}
            pending={pendingAction}
            onDismiss={onDismiss}
          />
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

function PermissionDeniedDetails({
  message,
}: {
  message: Extract<ChatItem, { type: 'permission_denied' }>
}) {
  return (
    <>
      <p className="permission-tool">{message.tool}</p>
      {message.command === undefined ? (
        <pre className="syntax-code-block permission-command permission-denied-input permission-denied-summary">
          {message.inputSummary}
        </pre>
      ) : (
        <SyntaxCodeBlock
          className="permission-command permission-denied-input"
          code={message.command}
          displayCode={formatPermissionCommand(message.command)}
          language="bash"
        />
      )}
      {message.reason === null ? null : (
        <p className="permission-denied-reason">{message.reason}</p>
      )}
    </>
  )
}

function DismissButton({
  canDismiss,
  pending,
  onDismiss,
}: {
  canDismiss: boolean
  pending: PendingPermissionActionState
  onDismiss: () => void
}) {
  const dismissAction = pending?.action === 'dismiss' ? pending : null
  return (
    <Button
      type="button"
      variant="plain"
      disabled={!canDismiss || pending !== null}
      aria-busy={dismissAction?.status === 'sending'}
      onClick={() => {
        onDismiss()
      }}
    >
      {dismissAction?.status === 'sending' ? 'Dismissing…' : 'Dismiss'}
    </Button>
  )
}
