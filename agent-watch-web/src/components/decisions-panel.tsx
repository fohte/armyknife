import '#components/decisions-panel.css'

import { useCallback, useEffect, useMemo, useState } from 'react'

import { type Decision, DecisionCard } from '#components/decision-card'
import type { Session } from '#types'

type DecisionsPanelProps = {
  decisions: readonly Decision[]
  session: Session
  selectedSeq: number | null
  relativeTimeReference?: number
  showReviewed?: boolean
  onSelect: (seq: number) => void
  onOk: (seq: number) => Promise<boolean>
  onChange: (seq: number, comment: string) => Promise<boolean>
}

function changeTargetName(session: Session) {
  if (session.category === 'done' && session.parent !== null) {
    return session.parent.title ?? 'parent session'
  }
  return session.title
}

function isEditableTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) ||
      target.closest('[role="textbox"]') !== null ||
      target.closest('[contenteditable="true"]') !== null)
  )
}

function isButtonTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    target.closest('button, [role="button"]') !== null
  )
}

export function DecisionsPanel({
  decisions,
  session,
  selectedSeq,
  relativeTimeReference = Date.now(),
  showReviewed = false,
  onSelect,
  onOk,
  onChange,
}: DecisionsPanelProps) {
  const [editingSeq, setEditingSeq] = useState<number | null>(null)
  const displayedDecisions = useMemo(
    () =>
      showReviewed
        ? decisions
        : decisions.filter((decision) => decision.review !== 'ok'),
    [decisions, showReviewed],
  )
  const selectedDecision =
    displayedDecisions.find((decision) => decision.seq === selectedSeq) ??
    displayedDecisions.find((decision) => decision.review === null) ??
    displayedDecisions[0]
  const reviewedCount = decisions.filter(
    (decision) => decision.review !== null,
  ).length
  const targetName = changeTargetName(session)
  const orderedDecisions = useMemo(
    () =>
      [...displayedDecisions].sort(
        (left, right) =>
          Number(left.review !== null) - Number(right.review !== null),
      ),
    [displayedDecisions],
  )
  const pendingSeqs = useMemo(
    () => orderedDecisions.filter((decision) => decision.review === null),
    [orderedDecisions],
  )
  const approveDecision = useCallback(
    async (seq: number) => {
      const success = await onOk(seq)
      if (!success) return false
      const nextDecision = pendingSeqs.find((decision) => decision.seq !== seq)
      if (nextDecision !== undefined) onSelect(nextDecision.seq)
      return true
    },
    [onOk, onSelect, pendingSeqs],
  )

  useEffect(() => {
    if (editingSeq === null) return
    document.getElementById(`decision-change-${String(editingSeq)}`)?.focus()
  }, [editingSeq])

  useEffect(() => {
    const onKeyDown = (event: WindowEventMap['keydown']) => {
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        (event.shiftKey && event.key.toLowerCase() !== 'c') ||
        isEditableTarget(event.target) ||
        selectedDecision === undefined
      ) {
        return
      }

      if (
        (event.key === 'Enter' || event.key === ' ') &&
        isButtonTarget(event.target)
      ) {
        return
      }

      if (
        event.key === 'Enter' &&
        event.target instanceof HTMLElement &&
        event.target.closest('a[href], [role="link"]') !== null
      ) {
        return
      }

      if (
        (event.key === 'y' || event.key === 'Y') &&
        selectedDecision.review === null
      ) {
        event.preventDefault()
        void approveDecision(selectedDecision.seq)
        return
      }

      if (
        (event.key === 'c' || event.key === 'C') &&
        event.shiftKey &&
        selectedDecision.review === null
      ) {
        event.preventDefault()
        setEditingSeq(selectedDecision.seq)
        return
      }

      if (event.key === 'Enter') {
        event.preventDefault()
        document
          .getElementById(`decision-row-${String(selectedDecision.seq)}`)
          ?.scrollIntoView({ behavior: 'smooth', block: 'center' })
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [approveDecision, selectedDecision])

  return (
    <section className="decisions-panel" aria-label="Decisions in this session">
      <div className="decisions-panel-list">
        <header className="decisions-panel-header">
          <h2>Decisions in this session</h2>
          <span>
            {String(reviewedCount)} / {String(decisions.length)} reviewed
          </span>
        </header>
        {orderedDecisions.length === 0 && decisions.length > 0 ? (
          <p className="decisions-panel-empty">All decisions are approved.</p>
        ) : (
          orderedDecisions.map((decision) => {
            const selected = decision.seq === selectedDecision?.seq
            return (
              <DecisionCard
                key={decision.seq}
                sessionId={session.id}
                decision={decision}
                variant="panel"
                relativeTimeReference={relativeTimeReference}
                expanded={selected}
                selected={selected}
                editing={editingSeq === decision.seq}
                targetName={targetName}
                onSelect={() => {
                  onSelect(decision.seq)
                }}
                onOk={() => approveDecision(decision.seq)}
                onBeginChange={() => {
                  onSelect(decision.seq)
                  setEditingSeq(decision.seq)
                }}
                onRequestChange={async (comment) => {
                  const success = await onChange(decision.seq, comment)
                  if (success) setEditingSeq(null)
                  return success
                }}
              />
            )
          })
        )}
      </div>
      <footer
        className="decisions-panel-footer"
        aria-label="Decision shortcuts"
      >
        <p className="decisions-panel-jump-hint">
          Tap a decision to jump to it in the conversation.
        </p>
        <span className="decisions-panel-keyboard-hint">
          Y OK and next · ⇧C Change · ↵ Jump to it in the conversation
        </span>
      </footer>
    </section>
  )
}
