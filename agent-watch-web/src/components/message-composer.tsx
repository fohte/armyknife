import '#components/message-composer.css'

import { Button } from '@fohte/ui/button'
import type { KeyboardEvent, SubmitEvent } from 'react'
import { useLayoutEffect, useRef } from 'react'

type MessageComposerProps = {
  draft: string
  placeholder: string
  disabled: boolean
  isSending: boolean
  isMobile: boolean
  canInterrupt: boolean
  onDraftChange: (draft: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void
  onSend: () => void
  onInterrupt: () => void
}

export function MessageComposer({
  draft,
  placeholder,
  disabled,
  isSending,
  isMobile,
  canInterrupt,
  onDraftChange,
  onKeyDown,
  onSend,
  onInterrupt,
}: MessageComposerProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  useLayoutEffect(() => {
    const textarea = textareaRef.current
    if (textarea === null) return

    textarea.style.height = 'auto'
    const maxHeight = Number.parseFloat(
      window.getComputedStyle(textarea).maxHeight,
    )
    const height = Number.isFinite(maxHeight)
      ? Math.min(textarea.scrollHeight, maxHeight)
      : textarea.scrollHeight
    textarea.style.height = `${String(height)}px`
    textarea.style.overflowY =
      textarea.scrollHeight > height ? 'auto' : 'hidden'
  }, [draft])

  const handleSubmit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    onSend()
  }

  return (
    <form
      className="conversation-composer"
      data-mobile={isMobile ? 'true' : undefined}
      onSubmit={handleSubmit}
    >
      <textarea
        ref={textareaRef}
        aria-label="Message"
        placeholder={placeholder}
        value={draft}
        onChange={(event) => {
          onDraftChange(event.currentTarget.value)
        }}
        onKeyDown={onKeyDown}
        disabled={disabled}
        rows={2}
      />
      <div className="composer-footer">
        {!isMobile ? (
          <span role={isSending ? 'status' : undefined}>
            {isSending ? 'Sending…' : '↵ Send · ⇧↵ New line · Esc Interrupt'}
          </span>
        ) : null}
        {isMobile && canInterrupt ? (
          <Button
            className="mobile-interrupt-button"
            type="button"
            variant="plain"
            onClick={onInterrupt}
          >
            Interrupt
          </Button>
        ) : null}
        <Button
          type="submit"
          variant="plain"
          aria-label="Send message"
          disabled={draft.trim() === '' || disabled || isSending}
        >
          ↑
        </Button>
      </div>
    </form>
  )
}
