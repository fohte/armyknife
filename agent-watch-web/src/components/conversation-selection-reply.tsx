import '#components/conversation-selection-reply.css'

import { Button } from '@fohte/ui/button'

export type ConversationSelectionReplyPosition = {
  left: number
  top: number
}

type ConversationSelectionReplyProps = {
  position: ConversationSelectionReplyPosition | null
  onReply: () => void
}

export function ConversationSelectionReply({
  position,
  onReply,
}: ConversationSelectionReplyProps) {
  if (position === null) return null

  return (
    <div
      className="conversation-selection-reply"
      role="toolbar"
      aria-label="Reply to selected text"
      style={{ left: position.left, top: position.top }}
    >
      <Button
        type="button"
        variant="plain"
        onMouseDown={(event) => {
          event.preventDefault()
        }}
        onClick={onReply}
      >
        Reply
      </Button>
    </div>
  )
}
