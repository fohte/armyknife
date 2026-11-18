import '#components/message-composer.css'

import { Button } from '@fohte/ui/button'
import { ArrowUp, CircleStop } from 'lucide-react'
import type { ClipboardEvent, KeyboardEvent, SubmitEvent } from 'react'
import { useLayoutEffect } from 'react'

import {
  type AttachmentPreview,
  AttachmentPreviewList,
  handleAttachmentPaste,
} from '#components/attachment-preview-list'
import {
  skillCompletionTextareaAttributes,
  SkillSuggestionList,
  useSkillCompletion,
} from '#components/skill-completion'
import type { Session, SkillSuggestion } from '#types'

type MessageComposerProps = {
  draft: string
  focusDraft?: string | null
  focusRequestId?: number | null
  onFocusRequestHandled?: (requestId: number) => void
  actionsReady?: boolean
  placeholder: string
  disabled: boolean
  isSending: boolean
  isProcessingAttachments: boolean
  isMobile: boolean
  canInterrupt: boolean
  isInterrupting?: boolean
  attachments: readonly ComposerAttachment[]
  engine?: Session['engine']
  skills?: readonly SkillSuggestion[]
  onDraftChange: (draft: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void
  onPasteFiles: (files: File[]) => void
  onRemoveAttachment: (id: string) => void
  onSend: () => void
  onInterrupt: () => void
}

type ComposerAttachment = AttachmentPreview

export function MessageComposer({
  draft,
  focusDraft = null,
  focusRequestId = null,
  onFocusRequestHandled,
  actionsReady = true,
  placeholder,
  disabled,
  isSending,
  isProcessingAttachments,
  isMobile,
  canInterrupt,
  isInterrupting = false,
  attachments,
  engine,
  skills = [],
  onDraftChange,
  onKeyDown,
  onPasteFiles,
  onRemoveAttachment,
  onSend,
  onInterrupt,
}: MessageComposerProps) {
  const skillCompletion = useSkillCompletion({
    draft,
    engine,
    skills,
    onDraftChange,
    onKeyDown,
  })

  useLayoutEffect(() => {
    const textarea = skillCompletion.textareaRef.current
    if (textarea === null) return

    textarea.style.height = 'auto'
    const maxHeight = Number.parseFloat(
      window.getComputedStyle(textarea).maxHeight,
    )
    const height = Number.isFinite(maxHeight)
      ? Math.min(textarea.scrollHeight, maxHeight)
      : textarea.scrollHeight
    textarea.style.height = `${String(height)}px`
    if (focusDraft === draft) {
      textarea.focus()
      textarea.setSelectionRange(draft.length, draft.length)
    }
  }, [draft, focusDraft])

  useLayoutEffect(() => {
    if (focusRequestId === null || !actionsReady) return
    const textarea = skillCompletion.textareaRef.current
    if (textarea !== null && !disabled && !isMobile) {
      textarea.focus()
    }
    onFocusRequestHandled?.(focusRequestId)
  }, [
    actionsReady,
    disabled,
    focusRequestId,
    isMobile,
    onFocusRequestHandled,
    skillCompletion.textareaRef,
  ])

  const handleSubmit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    onSend()
  }

  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    handleAttachmentPaste(event, onPasteFiles)
  }

  return (
    <form
      className="conversation-composer"
      aria-busy={isProcessingAttachments}
      data-actions-ready={actionsReady ? 'true' : 'false'}
      data-mobile={isMobile ? 'true' : undefined}
      onSubmit={handleSubmit}
    >
      <textarea
        ref={skillCompletion.textareaRef}
        aria-label="Message"
        placeholder={placeholder}
        value={draft}
        onChange={skillCompletion.handleChange}
        onSelect={skillCompletion.handleSelect}
        onKeyDown={skillCompletion.handleKeyDown}
        onPaste={handlePaste}
        disabled={disabled}
        rows={isMobile ? 1 : 2}
        {...skillCompletionTextareaAttributes({
          isOpen: skillCompletion.isOpen,
          id: skillCompletion.suggestionsId,
          selectedIndex: skillCompletion.selectedSkillIndex,
        })}
      />
      {skillCompletion.isOpen ? (
        <SkillSuggestionList
          id={skillCompletion.suggestionsId}
          completion={skillCompletion.completion}
          skills={skillCompletion.matchingSkills}
          selectedIndex={skillCompletion.selectedSkillIndex}
          onSelect={skillCompletion.selectSkill}
        />
      ) : null}
      <AttachmentPreviewList
        attachments={attachments}
        onRemove={onRemoveAttachment}
      />
      <div className="composer-footer">
        {!isMobile ? (
          <>
            <span
              role={
                isSending || isProcessingAttachments || isInterrupting
                  ? 'status'
                  : undefined
              }
            >
              {isSending
                ? 'Sending…'
                : isProcessingAttachments
                  ? 'Preparing attachments…'
                  : isInterrupting
                    ? 'Interrupting…'
                    : canInterrupt
                      ? '↵ Send · ⇧↵ / ⌥↵ New line · Esc Interrupt'
                      : '↵ Send · ⇧↵ / ⌥↵ New line'}
            </span>
            <span>⌥↑ / ⌥↓ My messages</span>
          </>
        ) : isProcessingAttachments ? (
          <span role="status">Preparing attachments…</span>
        ) : null}
        {isMobile && canInterrupt ? (
          <Button
            className="mobile-interrupt-button"
            type="button"
            variant="plain"
            aria-label={isInterrupting ? 'Interrupting…' : 'Interrupt'}
            aria-busy={isInterrupting}
            disabled={isInterrupting}
            onClick={onInterrupt}
          >
            <CircleStop aria-hidden="true" size={18} />
          </Button>
        ) : null}
        <Button
          type="submit"
          variant="plain"
          aria-label="Send message"
          disabled={
            (draft.trim() === '' && attachments.length === 0) ||
            disabled ||
            isSending ||
            isProcessingAttachments
          }
        >
          <ArrowUp aria-hidden="true" size={14} />
        </Button>
      </div>
    </form>
  )
}
