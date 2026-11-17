import '#components/decision-card.css'

import { Button } from '@fohte/ui/button'
import { Fragment, useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import { cardDraftStorageKey, useDraft } from '#components/use-composer-draft'
import type { ChatItem } from '#types'

export type Decision = Extract<ChatItem, { type: 'decision' }>

function formatRelativeTime(createdAt: string, referenceTime: number) {
  const createdAtMilliseconds = Date.parse(createdAt)
  if (!Number.isFinite(createdAtMilliseconds)) return 'unknown time'

  const elapsedMinutes = Math.floor(
    Math.max(0, referenceTime - createdAtMilliseconds) / 60_000,
  )
  if (elapsedMinutes === 0) return 'just now'
  if (elapsedMinutes < 60) return `${String(elapsedMinutes)}m ago`

  const elapsedHours = Math.floor(elapsedMinutes / 60)
  if (elapsedHours < 24) return `${String(elapsedHours)}h ago`
  return `${String(Math.floor(elapsedHours / 24))}d ago`
}

type DecisionCardProps = {
  sessionId?: string
  decision: Decision
  variant: 'conversation' | 'panel'
  expanded?: boolean
  selected?: boolean
  position?: string
  relativeTimeReference?: number
  editing?: boolean
  targetName?: string
  onToggleExpanded?: () => void
  onSelect?: () => void
  onOk: () => Promise<boolean>
  onBeginChange?: () => void
  onRequestChange: (comment: string) => Promise<boolean>
}

export function DecisionCard({
  sessionId,
  decision,
  variant,
  expanded: controlledExpanded,
  selected = false,
  position,
  relativeTimeReference = Date.now(),
  editing: controlledEditing,
  targetName,
  onToggleExpanded,
  onSelect,
  onOk,
  onBeginChange,
  onRequestChange,
}: DecisionCardProps) {
  const [localExpanded, setLocalExpanded] = useState(false)
  const [localEditing, setLocalEditing] = useState(false)
  const [draftSessionId] = useState(() => sessionId ?? crypto.randomUUID())
  const {
    draft: comment,
    updateDraft,
    clearDraftIfUnchanged,
    storageAvailable,
  } = useDraft(
    cardDraftStorageKey(
      'decision',
      draftSessionId,
      decision.seq,
      'change-comment',
    ),
  )
  const expanded = controlledExpanded ?? localExpanded
  const editing = controlledEditing ?? localEditing
  const detailsId = `decision-details-${variant}-${String(decision.seq)}`

  const toggleExpanded = () => {
    if (onToggleExpanded !== undefined) {
      onToggleExpanded()
    } else {
      setLocalExpanded((current) => !current)
    }
  }

  const beginChange = () => {
    if (onBeginChange !== undefined) {
      onBeginChange()
    } else {
      setLocalEditing(true)
    }
  }

  const submitChange = async () => {
    if (comment.trim() === '') return
    if (await onRequestChange(comment.trim())) {
      clearDraftIfUnchanged(comment)
      setLocalEditing(false)
    }
  }

  return (
    <article
      id={
        variant === 'conversation'
          ? `decision-row-${String(decision.seq)}`
          : undefined
      }
      className={`decision-card decision-card--${variant}`}
      data-selected={selected ? 'true' : undefined}
      data-review={decision.review ?? 'unreviewed'}
    >
      {variant === 'conversation' ? (
        <button
          className="decision-card-summary"
          type="button"
          aria-expanded={expanded}
          aria-controls={detailsId}
          onClick={() => {
            onSelect?.()
            toggleExpanded()
          }}
        >
          <span className="decision-card-symbol" aria-hidden="true">
            ◇
          </span>
          <span className="decision-card-summary-label">Decided</span>
          <strong className="decision-card-title">{decision.title}</strong>
          <span className="decision-card-considered-count">
            · also considered {String(decision.considered.length)}
          </span>
          {position !== undefined ? (
            <span className="decision-card-position">{position}</span>
          ) : null}
          <span className="decision-card-chevron" aria-hidden="true">
            {expanded ? '▾' : '▸'}
          </span>
        </button>
      ) : (
        <button
          className="decision-panel-card-heading"
          type="button"
          aria-expanded={expanded}
          onClick={() => {
            if (onSelect !== undefined) onSelect()
            else toggleExpanded()
          }}
        >
          <span className="decision-panel-card-kicker">
            ◇ Decided ·{' '}
            <time dateTime={decision.createdAt}>
              {formatRelativeTime(decision.createdAt, relativeTimeReference)}
            </time>
          </span>
          {!expanded ? (
            <>
              <strong className="decision-card-title">{decision.title}</strong>
              <span className="decision-panel-card-collapsed-considered">
                Also considered ·{' '}
                {decision.considered.map((item, index) => (
                  <Fragment key={String(index)}>
                    {index > 0 ? ' / ' : null}
                    <MarkdownMessage inline>{item}</MarkdownMessage>
                  </Fragment>
                ))}
              </span>
            </>
          ) : null}
        </button>
      )}

      {expanded ? (
        <div id={detailsId} className="decision-card-details">
          {variant === 'panel' ? (
            <h3 className="decision-panel-card-title">{decision.title}</h3>
          ) : null}
          <div className="decision-card-detail-field">
            <span>Why · </span>
            <MarkdownMessage>{decision.why}</MarkdownMessage>
          </div>
          <div className="decision-card-detail-field">
            <span>Also considered · </span>
            <div className="decision-card-considered">
              {decision.considered.map((item, index) => (
                <MarkdownMessage key={String(index)} inline>
                  {item}
                </MarkdownMessage>
              ))}
            </div>
          </div>
          <div className="decision-card-detail-field">
            <span>Easy to change · </span>
            <MarkdownMessage>{decision.easyToChange}</MarkdownMessage>
          </div>
          {decision.review === null ? (
            editing ? (
              <form
                className="decision-card-change-form"
                onSubmit={(event) => {
                  event.preventDefault()
                  void submitChange()
                }}
              >
                {!storageAvailable ? (
                  <p className="conversation-error" role="status">
                    Browser storage is unavailable. Drafts may not survive a
                    page reload.
                  </p>
                ) : null}
                <label htmlFor={`decision-change-${String(decision.seq)}`}>
                  Change request
                </label>
                <textarea
                  id={`decision-change-${String(decision.seq)}`}
                  value={comment}
                  onChange={(event) => {
                    updateDraft(event.currentTarget.value)
                  }}
                  rows={3}
                  autoFocus
                />
                <Button
                  className="aw-button aw-button--primary"
                  type="submit"
                  disabled={comment.trim() === ''}
                >
                  Send change request
                </Button>
              </form>
            ) : (
              <div className="decision-card-actions">
                <div className="decision-card-buttons">
                  <Button
                    className={
                      variant === 'panel'
                        ? 'aw-button aw-button--primary'
                        : 'aw-button aw-button--outline'
                    }
                    type="button"
                    onClick={() => void onOk()}
                  >
                    OK
                    {variant === 'panel' ? <kbd>Y</kbd> : null}
                  </Button>
                  <Button
                    className="aw-button aw-button--outline"
                    type="button"
                    onClick={beginChange}
                  >
                    {variant === 'panel' ? (
                      <>
                        Change…{' '}
                        <span className="decision-card-target-name">
                          (sends to {targetName ?? 'this session'})
                        </span>
                        <kbd>C</kbd>
                      </>
                    ) : (
                      'Change…'
                    )}
                  </Button>
                </div>
                {variant === 'conversation' ? (
                  <span className="decision-card-not-blocking">
                    Not blocking · keeps working
                  </span>
                ) : null}
              </div>
            )
          ) : (
            <p className="decision-card-review-state">
              {decision.review === 'ok' ? 'Reviewed · OK' : 'Change requested'}
            </p>
          )}
        </div>
      ) : null}
    </article>
  )
}
