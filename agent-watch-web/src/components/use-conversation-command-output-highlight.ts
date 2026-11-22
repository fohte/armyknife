import { useEffect, useRef } from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import type { ChatItem } from '#types'

type UseConversationCommandOutputHighlightOptions = {
  loading: boolean
  messages: readonly ChatItem[]
  scrollRef: { current: HTMLDivElement | null }
  sessionId: string
}

export function useConversationCommandOutputHighlight({
  loading,
  messages,
  scrollRef,
  sessionId,
}: UseConversationCommandOutputHighlightOptions) {
  const feedback = useOptionalAsyncActionFeedback()
  const timer = useRef<number | null>(null)
  const action = feedback?.actions.find(
    (candidate) =>
      feedback.pendingCommandOutputActionIds.includes(candidate.id) &&
      candidate.kind === 'command' &&
      candidate.sessionId === sessionId,
  )

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current)
    },
    [],
  )
  useEffect(() => {
    if (loading || action?.commandOutputIndex === undefined) return

    const target = scrollRef.current?.querySelector<HTMLElement>(
      `[data-command-result="true"][data-conversation-index="${String(action.commandOutputIndex)}"]`,
    )
    if (target === undefined || target === null) return

    target.dataset['commandOutputHighlighted'] = 'true'
    target.scrollIntoView({ behavior: 'smooth', block: 'center' })
    if (timer.current !== null) window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      delete target.dataset['commandOutputHighlighted']
      timer.current = null
    }, 1_500)
    feedback?.consumeCommandOutput(action.id)
  }, [action, feedback, loading, messages, scrollRef])
}
