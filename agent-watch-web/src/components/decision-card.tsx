import '#components/decision-card.css'

import { Button } from '@fohte/ui/button'
import { useState } from 'react'

import type { ChatItem } from '#types'

export type Decision = Extract<ChatItem, { type: 'decision' }>

function formatRelativeTime(createdAt: string) {
  const createdAtMilliseconds = Date.parse(createdAt)
  if (!Number.isFinite(createdAtMilliseconds)) return 'unknown time'

  const elapsedMinutes = Math.floor(
    Math.max(0, Date.now() - createdAtMilliseconds) / 60_000,
  )
  if (elapsedMinutes === 0) return 'just now'
  if (elapsedMinutes < 60) return `${String(elapsedMinutes)}m ago`

  const elapsedHours = Math.floor(elapsedMinutes / 60)
  if (elapsedHours < 24) return `${String(elapsedHours)}h ago`
  return `${String(Math.floor(elapsedHours / 24))}d ago`
}

type DecisionCardProps = {
  decision: Decision
  variant: 'conversation' | 'panel'
  expanded?: boolean
  selected?: boolean
  position?: string
  editing?: boolean
  targetName?: string
  onToggleExpanded?: () => void
  onSelect?: () => void
  onOk: () => Promise<boolean>
  onBeginChange?: () => void
  onRequestChange: (comment: string) => Promise<boolean>
}

export function DecisionCard({
  decision,
  variant,
  expanded: controlledExpanded,
  selected = false,
  position,
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
  const [comment, setComment] = useState('')
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
      setComment('')
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
              {formatRelativeTime(decision.createdAt)}
            </time>
          </span>
          {!expanded ? (
            <>
              <strong className="decision-card-title">{decision.title}</strong>
              <span className="decision-panel-card-collapsed-considered">
                Also considered · {decision.considered.join(' / ')}
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
          <p>
            <span>Why · </span>
            {decision.why}
          </p>
          <p>
            <span>Also considered · </span>
            {decision.considered.join(' / ')}
          </p>
          <p>
            <span>Easy to change · </span>
            {decision.easyToChange}
          </p>
          {decision.review === null ? (
            editing ? (
              <form
                className="decision-card-change-form"
                onSubmit={(event) => {
                  event.preventDefault()
                  void submitChange()
                }}
              >
                <label htmlFor={`decision-change-${String(decision.seq)}`}>
                  Change request
                </label>
                <textarea
                  id={`decision-change-${String(decision.seq)}`}
                  value={comment}
                  onChange={(event) => {
                    setComment(event.currentTarget.value)
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
                        <span>(sends to {targetName ?? 'this session'})</span>
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
