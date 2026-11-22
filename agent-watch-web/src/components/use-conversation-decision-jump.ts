import { useEffect, useRef } from 'react'

import type { ChatItem } from '#types'

type UseConversationDecisionJumpOptions = {
  decisionJump: { seq: number } | null
  epoch: string | undefined
  loadingOlder: boolean
  loadOlder: () => void
  messages: readonly ChatItem[]
  olderCursor: string | undefined
  onComplete: () => void
  onSucceeded: () => void
  start: number
}

export function useConversationDecisionJump({
  decisionJump,
  epoch,
  loadingOlder,
  loadOlder,
  messages,
  olderCursor,
  onComplete,
  onSucceeded,
  start,
}: UseConversationDecisionJumpOptions) {
  const loadAttempt = useRef<{
    epoch: string | undefined
    olderCursor: string | undefined
    start: number
  } | null>(null)

  useEffect(() => {
    if (decisionJump === null) {
      loadAttempt.current = null
      return
    }
    const decisionRow = document.getElementById(
      `decision-row-${String(decisionJump.seq)}`,
    )
    if (decisionRow !== null) {
      loadAttempt.current = null
      decisionRow.scrollIntoView({ block: 'center' })
      onSucceeded()
      return
    }
    const previousAttempt = loadAttempt.current
    if (previousAttempt !== null) {
      if (loadingOlder) return
      const pageAdvanced =
        previousAttempt.start !== start ||
        previousAttempt.epoch !== epoch ||
        previousAttempt.olderCursor !== olderCursor
      loadAttempt.current = null
      if (!pageAdvanced) {
        onComplete()
        return
      }
    }
    if (start > 0 || (epoch !== undefined && olderCursor != null)) {
      loadAttempt.current = {
        epoch,
        olderCursor,
        start,
      }
      loadOlder()
      return
    }
    loadAttempt.current = null
    onComplete()
  }, [
    decisionJump,
    epoch,
    loadOlder,
    loadingOlder,
    messages,
    olderCursor,
    onComplete,
    onSucceeded,
    start,
  ])
}
