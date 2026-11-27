import '#components/message-composer.css'

import { Button } from '@fohte/ui/button'
import { CircleStop } from 'lucide-react'
import type { ClipboardEvent, KeyboardEvent, SubmitEvent } from 'react'
import { useLayoutEffect, useRef } from 'react'

import {
  type AttachmentPreview,
  handleAttachmentPaste,
} from '#components/attachment-preview-list'
import { Composer } from '#components/composer'
import {
  skillCompletionTextareaAttributes,
  SkillSuggestionList,
  useSkillCompletion,
} from '#components/skill-completion'
import { useDismissableLayer } from '#components/use-dismissable-layer'
import type { Session, SkillSuggestion } from '#types'

type MessageComposerProps = {
  draft: string
  focusDraft?: string | null
  focusRequestId?: number | null
  onFocusRequestHandled?: (requestId: number) => void
  actionsSettled?: boolean
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
  actionsSettled = true,
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
  const fileInputRef = useRef<HTMLInputElement>(null)
  const isComposingRef = useRef(false)
  const discardCompositionDraftRef = useRef(false)
  const discardNextCompositionChangeRef = useRef<string | null>(null)
  const handleDraftChange = (value: string) => {
    if (discardCompositionDraftRef.current) return
    if (discardNextCompositionChangeRef.current !== null) {
      const discardedDraft = discardNextCompositionChangeRef.current
      discardNextCompositionChangeRef.current = null
      if (value === discardedDraft) return
    }
    onDraftChange(value)
  }
  const skillCompletion = useSkillCompletion({
    draft,
    engine,
    skills,
    onDraftChange: handleDraftChange,
    onKeyDown,
  })
  useDismissableLayer(skillCompletion.isOpen, skillCompletion.dismiss, [
    skillCompletion.textareaRef,
    skillCompletion.suggestionsRef,
  ])

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
    if (focusRequestId === null || !actionsSettled) return
    const textarea = skillCompletion.textareaRef.current
    if (textarea !== null && !disabled && !isMobile) {
      textarea.focus()
    }
    onFocusRequestHandled?.(focusRequestId)
  }, [
    actionsSettled,
    disabled,
    focusRequestId,
    isMobile,
    onFocusRequestHandled,
    skillCompletion.textareaRef,
  ])

  const handleSubmit = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (isComposingRef.current) {
      discardCompositionDraftRef.current = true
      discardNextCompositionChangeRef.current = draft
    }
    onSend()
  }

  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    handleAttachmentPaste(event, onPasteFiles)
  }

  const statusText = isSending
    ? 'Sending…'
    : isProcessingAttachments
      ? 'Preparing attachments…'
      : isInterrupting
        ? 'Interrupting…'
        : undefined
  const sendState = disabled
    ? 'off'
    : (draft.trim() === '' && attachments.length === 0) ||
        isSending ||
        isProcessingAttachments
      ? 'dim'
      : 'on'
  const keyHint = isMobile
    ? undefined
    : `↵ Send · ⇧↵ / ⌥↵ New line${canInterrupt && !isInterrupting ? ' · Esc Interrupt' : ''}`
  const keyHint2 = isMobile ? undefined : '⌥↑ / ⌥↓ My messages'

  return (
    <form
      className="conversation-composer"
      aria-busy={isProcessingAttachments}
      data-actions-settled={actionsSettled ? 'true' : 'false'}
      data-mobile={isMobile ? 'true' : undefined}
      onSubmit={handleSubmit}
    >
      <Composer
        attachments={attachments}
        onRemoveAttachment={onRemoveAttachment}
        onAddAttachments={() => fileInputRef.current?.click()}
        sendState={sendState}
        keyHint={keyHint}
        keyHint2={keyHint2}
        statusText={statusText ?? ''}
        isMobile={isMobile}
        isBusy={isProcessingAttachments}
        trailingAction={
          isMobile && canInterrupt ? (
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
          ) : null
        }
      >
        <textarea
          ref={skillCompletion.textareaRef}
          aria-label="Message"
          placeholder={placeholder}
          value={draft}
          onChange={skillCompletion.handleChange}
          onCompositionStart={() => {
            isComposingRef.current = true
            discardNextCompositionChangeRef.current = null
          }}
          onCompositionEnd={() => {
            isComposingRef.current = false
            discardCompositionDraftRef.current = false
          }}
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
            ref={skillCompletion.suggestionsRef}
            id={skillCompletion.suggestionsId}
            completion={skillCompletion.completion}
            skills={skillCompletion.matchingSkills}
            selectedIndex={skillCompletion.selectedSkillIndex}
            onSelect={skillCompletion.selectSkill}
          />
        ) : null}
      </Composer>
      <input
        ref={fileInputRef}
        type="file"
        hidden
        multiple
        onChange={(event) => {
          const files = Array.from(event.currentTarget.files ?? [])
          if (files.length > 0) onPasteFiles(files)
          event.currentTarget.value = ''
          if (files.length > 0) skillCompletion.textareaRef.current?.focus()
        }}
      />
    </form>
  )
}
