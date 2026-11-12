import '#components/message-composer.css'

import { Button } from '@fohte/ui/button'
import { ArrowUp } from 'lucide-react'
import type { ClipboardEvent, KeyboardEvent, SubmitEvent } from 'react'
import { useId, useLayoutEffect, useRef, useState } from 'react'

import type { Session, SkillSuggestion } from '#types'

type MessageComposerProps = {
  draft: string
  placeholder: string
  disabled: boolean
  isSending: boolean
  isProcessingAttachments: boolean
  isMobile: boolean
  canInterrupt: boolean
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
  isProcessingAttachments,
  isMobile,
  canInterrupt,
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
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const pendingCursorPosition = useRef<number | null>(null)
  const suggestionsId = useId()
  const [activeSkillIndex, setActiveSkillIndex] = useState(0)
  const [dismissedCompletion, setDismissedCompletion] = useState<string | null>(
    null,
  )
  const [caret, setCaret] = useState<{
    draft: string
    position: number
  } | null>(null)

  const cursorPosition = caret?.draft === draft ? caret.position : draft.length
  const completion = findSkillCompletion(draft, engine, cursorPosition)
  const matchingSkills =
    completion === null
      ? []
      : skills.filter((skill) =>
          skill.name
            .toLocaleLowerCase()
            .startsWith(completion.query.toLocaleLowerCase()),
        )
  const completionKey =
    completion === null
      ? null
      : `${String(completion.start)}:${completion.trigger}:${completion.query}`
  const isCompletionOpen =
    completion !== null &&
    matchingSkills.length > 0 &&
    completionKey !== dismissedCompletion
  const selectedSkillIndex = Math.min(
    activeSkillIndex,
    Math.max(0, matchingSkills.length - 1),
  )

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
    if (pendingCursorPosition.current !== null) {
      const cursor = pendingCursorPosition.current
      pendingCursorPosition.current = null
      textarea.focus()
      textarea.setSelectionRange(cursor, cursor)
    }
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

  const selectSkill = (skill: SkillSuggestion) => {
    if (completion === null || completionKey === null) return
    const afterToken = draft.slice(completion.end)
    const separator = /^\s/.test(afterToken) ? '' : ' '
    const insertion = `${completion.trigger}${skill.name}${separator}`
    const nextDraft = draft.slice(0, completion.start) + insertion + afterToken
    const nextCursor = completion.start + insertion.length
    if (nextDraft === draft) {
      pendingCursorPosition.current = null
      const textarea = textareaRef.current
      textarea?.focus()
      textarea?.setSelectionRange(nextCursor, nextCursor)
    } else {
      pendingCursorPosition.current = nextCursor
    }
    setCaret({ draft: nextDraft, position: nextCursor })
    onDraftChange(nextDraft)
    setDismissedCompletion(
      `${String(completion.start)}:${completion.trigger}:${skill.name}`,
    )
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (
      isCompletionOpen &&
      !event.nativeEvent.isComposing &&
      Reflect.get(event.nativeEvent, 'keyCode') !== 229
    ) {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        const direction = event.key === 'ArrowDown' ? 1 : -1
        setActiveSkillIndex(
          (index) =>
            (index + direction + matchingSkills.length) % matchingSkills.length,
        )
        return
      }
      if (event.key === 'Enter' || event.key === 'Tab') {
        const skill = matchingSkills[selectedSkillIndex]
        if (skill !== undefined) {
          event.preventDefault()
          selectSkill(skill)
          return
        }
      }
      if (event.key === 'Escape' && completionKey !== null) {
        event.preventDefault()
        setDismissedCompletion(completionKey)
        return
      }
    }
    onKeyDown(event)
  }

  return (
    <form
      className="conversation-composer"
      aria-busy={isProcessingAttachments}
      data-mobile={isMobile ? 'true' : undefined}
      onSubmit={handleSubmit}
    >
      <textarea
        ref={textareaRef}
        aria-label="Message"
        placeholder={placeholder}
        value={draft}
        onChange={(event) => {
          const nextDraft = event.currentTarget.value
          setCaret({
            draft: nextDraft,
            position: event.currentTarget.selectionStart,
          })
          setDismissedCompletion(null)
          setActiveSkillIndex(0)
          onDraftChange(nextDraft)
        }}
        onSelect={(event) => {
          setCaret({
            draft: event.currentTarget.value,
            position: event.currentTarget.selectionStart,
          })
        }}
        onKeyDown={handleKeyDown}
        onPaste={handlePaste}
        disabled={disabled}
        rows={isMobile ? 1 : 2}
        aria-autocomplete={isCompletionOpen ? 'list' : undefined}
        aria-expanded={isCompletionOpen}
        aria-controls={isCompletionOpen ? suggestionsId : undefined}
        aria-activedescendant={
          isCompletionOpen
            ? `${suggestionsId}-skill-${String(selectedSkillIndex)}`
            : undefined
        }
      />
      {isCompletionOpen ? (
        <ul
          id={suggestionsId}
          className="composer-skill-suggestions"
          role="listbox"
          aria-label="Available skills"
        >
          {matchingSkills.map((skill, index) => (
            <li key={skill.name} role="presentation">
              <button
                id={`${suggestionsId}-skill-${String(index)}`}
                className="composer-skill-option"
                type="button"
                role="option"
                aria-selected={index === selectedSkillIndex}
                onMouseDown={(event) => {
                  event.preventDefault()
                }}
                onClick={() => {
                  selectSkill(skill)
                }}
              >
                <span>
                  {completion.trigger}
                  {skill.name}
                </span>
                <small>{skill.description}</small>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
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
          <span
            role={isSending || isProcessingAttachments ? 'status' : undefined}
          >
            {isSending
              ? 'Sending…'
              : isProcessingAttachments
                ? 'Preparing attachments…'
                : canInterrupt
                  ? '↵ Send · ⇧↵ / ⌥↵ New line · Esc Interrupt'
                  : '↵ Send · ⇧↵ / ⌥↵ New line'}
          </span>
        ) : isProcessingAttachments ? (
          <span role="status">Preparing attachments…</span>
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

function findSkillCompletion(
  draft: string,
  engine: Session['engine'],
  cursor: number,
) {
  if (engine !== 'Claude' && engine !== 'Codex') return null
  const beforeCursor = draft.slice(0, cursor)
  const match = /(?:^|\s)([/$])([^\s]*)$/.exec(beforeCursor)
  if (match === null || match[1] === undefined || match[2] === undefined) {
    return null
  }
  const trigger = match[1]
  if (
    (engine === 'Claude' && trigger !== '/') ||
    (engine === 'Codex' && trigger !== '$')
  ) {
    return null
  }
  const start = cursor - match[0].length + match[0].lastIndexOf(trigger)
  let end = cursor
  while (end < draft.length && !/\s/.test(draft[end] ?? '')) end += 1
  return { trigger, query: match[2], start, end }
}
