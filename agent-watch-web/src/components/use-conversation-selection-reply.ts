import { useCallback, useEffect, useState } from 'react'

import type { ConversationSelectionReplyPosition } from '#components/conversation-selection-reply'

type SelectionReply = {
  text: string
  position: ConversationSelectionReplyPosition
}

type UseConversationSelectionReplyOptions = {
  draft: string
  isMobile: boolean
  updateDraft: (draft: string) => void
}

export function useConversationSelectionReply({
  draft,
  isMobile,
  updateDraft,
}: UseConversationSelectionReplyOptions) {
  const [focusDraft, setFocusDraft] = useState<string | null>(null)
  const [selectionReply, setSelectionReply] = useState<SelectionReply | null>(
    null,
  )

  const updateSelectionReply = useCallback(() => {
    const selection = window.getSelection()
    if (selection === null || selection.isCollapsed) {
      setSelectionReply(null)
      return
    }

    const text = selection.toString()
    const range = selection.getRangeAt(0)
    const startNode =
      range.startContainer instanceof Element
        ? range.startContainer
        : range.startContainer.parentElement
    const endNode =
      range.endContainer instanceof Element
        ? range.endContainer
        : range.endContainer.parentElement
    const message = startNode?.closest('.chat-assistant-message')
    if (
      text.trim() === '' ||
      message === null ||
      message === undefined ||
      message !== endNode?.closest('.chat-assistant-message')
    ) {
      setSelectionReply(null)
      return
    }

    const rects = Array.from(range.getClientRects()).filter(
      (rect) => rect.width > 0 && rect.height > 0,
    )
    const visibleRects = rects.filter(
      (rect) => rect.bottom > 0 && rect.top < window.innerHeight,
    )
    const measuredRects = visibleRects.length > 0 ? visibleRects : rects
    const rect =
      measuredRects[Math.floor(measuredRects.length / 2)] ??
      range.getBoundingClientRect()
    const menuWidth = 88
    const menuHeight = 42
    const edge = 8
    const left = Math.min(
      Math.max(edge, rect.left + rect.width / 2 - menuWidth / 2),
      Math.max(edge, window.innerWidth - menuWidth - edge),
    )
    const below = rect.bottom + edge
    const above = rect.top - menuHeight - edge
    const fits = (top: number) =>
      top >= edge && top + menuHeight <= window.innerHeight - edge
    const preferred = isMobile ? below : above
    const alternate = isMobile ? above : below
    const top = fits(preferred)
      ? preferred
      : fits(alternate)
        ? alternate
        : Math.min(
            Math.max(preferred, edge),
            Math.max(edge, window.innerHeight - menuHeight - edge),
          )

    setSelectionReply({ text, position: { left, top } })
  }, [isMobile])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setSelectionReply(null)
    }
    document.addEventListener('selectionchange', updateSelectionReply)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('selectionchange', updateSelectionReply)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [updateSelectionReply])

  const dismiss = useCallback(() => {
    setSelectionReply(null)
  }, [])

  const reply = useCallback(() => {
    if (selectionReply === null) return
    const quotedText = selectionReply.text
      .replace(/\r\n?/g, '\n')
      .split('\n')
      .map((line) => `> ${line}`)
      .join('\n')
    const separator =
      draft === '' || draft.endsWith('\n\n')
        ? ''
        : draft.endsWith('\n')
          ? '\n'
          : '\n\n'
    const nextDraft = `${draft}${separator}${quotedText}\n`
    setFocusDraft(nextDraft)
    updateDraft(nextDraft)
    setSelectionReply(null)
  }, [draft, selectionReply, updateDraft])

  return {
    focusDraft,
    position: selectionReply?.position ?? null,
    dismiss,
    reply,
  }
}
