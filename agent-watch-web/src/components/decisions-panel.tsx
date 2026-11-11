import '#components/decisions-panel.css'

import { useEffect, useMemo, useState } from 'react'

import { type Decision, DecisionCard } from '#components/decision-card'
import type { Session } from '#types'

type DecisionsPanelProps = {
  decisions: readonly Decision[]
  session: Session
  selectedSeq: number | null
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

function isInteractiveTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) ||
      target.closest(
        'button, a[href], input, textarea, select, [role="button"], [role="tab"]',
      ) !== null)
  )
}

export function DecisionsPanel({
  decisions,
  session,
  selectedSeq,
  onSelect,
  onOk,
  onChange,
}: DecisionsPanelProps) {
  const [editingSeq, setEditingSeq] = useState<number | null>(null)
  const selectedDecision =
    decisions.find((decision) => decision.seq === selectedSeq) ??
    decisions.find((decision) => decision.review === null) ??
    decisions[0]
  const reviewedCount = decisions.filter(
    (decision) => decision.review !== null,
  ).length
  const targetName = changeTargetName(session)
  const pendingSeqs = useMemo(
    () => decisions.filter((decision) => decision.review === null),
    [decisions],
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
        event.shiftKey ||
        isInteractiveTarget(event.target) ||
        selectedDecision === undefined
      ) {
        return
      }

      if (
        (event.key === 'y' || event.key === 'Y') &&
        selectedDecision.review === null
      ) {
        event.preventDefault()
        void onOk(selectedDecision.seq).then((success) => {
          if (!success) return
          const nextDecision = pendingSeqs.find(
            (seq) => seq.seq !== selectedDecision.seq,
          )
          if (nextDecision !== undefined) onSelect(nextDecision.seq)
        })
        return
      }

      if (
        (event.key === 'c' || event.key === 'C') &&
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
  }, [onOk, onSelect, pendingSeqs, selectedDecision])

  return (
    <section className="decisions-panel" aria-label="Decisions in this session">
      <header className="decisions-panel-header">
        <h2>Decisions in this session</h2>
        <span>
          {String(reviewedCount)} / {String(decisions.length)} reviewed
        </span>
      </header>
      <div className="decisions-panel-list">
        {decisions.map((decision) => {
          const selected = decision.seq === selectedDecision?.seq
          return (
            <DecisionCard
              key={decision.seq}
              decision={decision}
              variant="panel"
              expanded={selected}
              selected={selected}
              editing={editingSeq === decision.seq}
              targetName={targetName}
              onSelect={() => {
                onSelect(decision.seq)
              }}
              onOk={() => onOk(decision.seq)}
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
        })}
      </div>
      <footer
        className="decisions-panel-footer"
        aria-label="Decision shortcuts"
      >
        <span>Y OK and next</span>
        <span>C Change</span>
        <span>↵ Jump to it in the conversation</span>
      </footer>
    </section>
  )
}
