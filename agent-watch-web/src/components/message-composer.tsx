import '#components/message-composer.css'

import { Button } from '@fohte/ui/button'
import type { ClipboardEvent, KeyboardEvent, SubmitEvent } from 'react'
import { useLayoutEffect, useRef } from 'react'

type MessageComposerProps = {
  draft: string
  placeholder: string
  disabled: boolean
  isSending: boolean
  isMobile: boolean
  canInterrupt: boolean
  attachments: readonly ComposerAttachment[]
  onDraftChange: (draft: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void
  onPasteFiles: (files: File[]) => void
  onRemoveAttachment: (id: string) => void
  onSend: () => void
  onInterrupt: () => void
}

export type ComposerAttachment = {
  id: string
  name: string
  size: number
  previewUrl: string | null
}

function formatFileSize(bytes: number) {
  if (bytes < 1024) return `${String(bytes)} B`
  const kibibytes = bytes / 1024
  const formatted = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
  })
  if (kibibytes < 1024) return `${formatted.format(kibibytes)} KiB`
  return `${formatted.format(kibibytes / 1024)} MiB`
}

export function MessageComposer({
  draft,
  placeholder,
  disabled,
  isSending,
  isMobile,
  canInterrupt,
  attachments,
  onDraftChange,
  onKeyDown,
  onPasteFiles,
  onRemoveAttachment,
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
  }, [draft])

  const handleSubmit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    onSend()
  }

  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData.files)
    if (files.length === 0) return
    event.preventDefault()
    onPasteFiles(files)
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
        onPaste={handlePaste}
        disabled={disabled}
        rows={isMobile ? 1 : 2}
      />
      {attachments.length > 0 ? (
        <ul className="composer-attachments" aria-label="Attachments">
          {attachments.map((attachment) => (
            <li className="composer-attachment" key={attachment.id}>
              {attachment.previewUrl === null ? null : (
                <img src={attachment.previewUrl} alt="" />
              )}
              <span>{attachment.name}</span>
              {attachment.previewUrl === null ? (
                <small>{formatFileSize(attachment.size)}</small>
              ) : null}
              <Button
                type="button"
                variant="plain"
                aria-label={`Remove ${attachment.name}`}
                onClick={() => {
                  onRemoveAttachment(attachment.id)
                }}
              >
                ×
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="composer-footer">
        {!isMobile ? (
          <span role={isSending ? 'status' : undefined}>
            {isSending
              ? 'Sending…'
              : canInterrupt
                ? '↵ Send · ⇧↵ New line · Esc Interrupt'
                : '↵ Send · ⇧↵ New line'}
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
          disabled={
            (draft.trim() === '' && attachments.length === 0) ||
            disabled ||
            isSending
          }
        >
          ↑
        </Button>
      </div>
    </form>
  )
}
