import '#components/proposal-card.css'

import { Button } from '@fohte/ui/button'
import type { ClipboardEvent } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'

import {
  AttachmentPreviewList,
  handleAttachmentPaste,
} from '#components/attachment-preview-list'
import { numberShortcutChoice } from '#components/conversation-keyboard'
import { MarkdownMessage } from '#components/markdown-message'
import type { EncodedAttachment } from '#components/use-attachments'
import { useAttachments } from '#components/use-attachments'
import { cardDraftStorageKey, useDraft } from '#components/use-composer-draft'
import type { ProposalAnswer, ProposalOption } from '#types'

type ProposalCardProps = {
  sessionId?: string
  seq: number
  title: string
  summary: string | null
  options: ProposalOption[]
  answer: ProposalAnswer | null
  resolved: boolean
  resolution: 'withdrawn' | null
  withdrawalComment: string | null
  isMobile: boolean
  defaultChoice?: number | null
  defaultComment?: string
  onAnswer: (
    answer: ProposalAnswer,
    attachments?: readonly EncodedAttachment[],
  ) => Promise<boolean>
}

function choiceLabel(choice: number) {
  return String.fromCharCode(65 + choice)
}

export function ProposalCard({
  sessionId,
  seq,
  title,
  summary,
  options,
  answer,
  resolved,
  resolution,
  withdrawalComment,
  isMobile,
  defaultChoice = null,
  defaultComment = '',
  onAnswer,
}: ProposalCardProps) {
  const [draftSessionId] = useState(() => sessionId ?? crypto.randomUUID())
  const attachmentState = useAttachments(
    `${sessionId ?? 'proposal'}:proposal:${String(seq)}`,
  )
  const commentDraft = useDraft(
    cardDraftStorageKey('proposal', draftSessionId, seq, 'comment'),
    answer?.comment ?? defaultComment,
  )
  const choiceDraft = useDraft(
    cardDraftStorageKey('proposal', draftSessionId, seq, 'choice'),
    defaultChoice === null ? '' : String(defaultChoice),
  )
  const [submittedAnswer, setSubmittedAnswer] = useState<ProposalAnswer | null>(
    null,
  )
  const latestAnswer = answer ?? submittedAnswer
  const savedChoice = Number(choiceDraft.draft)
  const selectedChoice =
    latestAnswer !== null
      ? latestAnswer.choice
      : choiceDraft.draft !== '' &&
          Number.isInteger(savedChoice) &&
          savedChoice >= 0 &&
          savedChoice < options.length
        ? savedChoice
        : null
  const setSelectedChoice = useCallback(
    (choice: number | null) => {
      choiceDraft.updateDraft(choice === null ? '' : String(choice))
    },
    [choiceDraft.updateDraft],
  )
  const { draft: comment, updateDraft, clearDraftIfUnchanged } = commentDraft
  const [isSubmitting, setIsSubmitting] = useState(false)
  const cardRef = useRef<HTMLElement | null>(null)
  const isWithdrawn = resolution === 'withdrawn'
  const isAnswered = latestAnswer !== null || resolved || isWithdrawn
  const isDisabled = isAnswered || isSubmitting

  useEffect(() => {
    if (answer !== null || resolved || isWithdrawn) {
      setSelectedChoice(answer?.choice ?? null)
    }
  }, [answer, resolved, isWithdrawn, setSelectedChoice])

  const submitAnswer = useCallback(
    async (choice: number | null) => {
      if (
        isDisabled ||
        attachmentState.isProcessingAttachments ||
        attachmentState.hasLimitError ||
        (choice !== null && !options[choice])
      ) {
        return
      }
      const response = {
        choice,
        comment: comment.trim() === '' ? null : comment.trim(),
      }
      setIsSubmitting(true)
      try {
        await attachmentState.sendWithAttachments(async (attachments) => {
          const succeeded =
            attachments.length === 0
              ? await onAnswer(response)
              : await onAnswer(response, attachments)
          if (succeeded) {
            setSubmittedAnswer(response)
            clearDraftIfUnchanged(comment)
            choiceDraft.clearDraftIfUnchanged(choiceDraft.draft)
          }
          return succeeded
        })
      } finally {
        setIsSubmitting(false)
      }
    },
    [
      attachmentState,
      choiceDraft,
      clearDraftIfUnchanged,
      comment,
      isDisabled,
      onAnswer,
      options,
    ],
  )

  useEffect(() => {
    const onKeyDown = (event: WindowEventMap['keydown']) => {
      const target = event.target
      const targetElement = target instanceof HTMLElement ? target : null
      const targetCard = targetElement?.closest('.chat-proposal-card') ?? null
      if (
        isDisabled ||
        event.defaultPrevented ||
        (targetCard !== null && targetCard !== cardRef.current)
      ) {
        return
      }

      if (event.metaKey && event.key === 'Enter') {
        if (
          event.altKey ||
          event.ctrlKey ||
          event.shiftKey ||
          targetCard !== cardRef.current ||
          selectedChoice === null
        ) {
          return
        }
        event.preventDefault()
        void submitAnswer(selectedChoice)
        return
      }

      if (targetCard === null) {
        const answerableCards = document.querySelectorAll(
          '.chat-proposal-card[data-answerable="true"]',
        )
        const latestAnswerableCard = answerableCards.item(
          answerableCards.length - 1,
        )
        if (latestAnswerableCard !== cardRef.current) return
      }

      const choice = numberShortcutChoice(event, options.length)
      if (choice === null) return
      event.preventDefault()
      setSelectedChoice(choice)
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [
    isDisabled,
    options.length,
    selectedChoice,
    setSelectedChoice,
    submitAnswer,
  ])

  const status =
    latestAnswer !== null
      ? latestAnswer.choice === null
        ? 'asked for another option'
        : `chose ${choiceLabel(latestAnswer.choice)}`
      : isWithdrawn
        ? 'withdrawn'
        : resolved
          ? 'answered elsewhere'
          : 'waiting for your choice'

  return (
    <article
      className="chat-proposal-card"
      aria-label="Proposal"
      aria-busy={isSubmitting || attachmentState.isProcessingAttachments}
      data-mobile={isMobile ? 'true' : undefined}
      data-answerable={isDisabled ? undefined : 'true'}
      ref={cardRef}
    >
      <h2>Proposal · {status}</h2>
      <h3>{title}</h3>
      {summary !== null ? (
        <MarkdownMessage className="chat-proposal-summary">
          {summary}
        </MarkdownMessage>
      ) : null}
      {isWithdrawn && withdrawalComment !== null ? (
        <MarkdownMessage className="chat-proposal-withdrawal-comment">
          {withdrawalComment}
        </MarkdownMessage>
      ) : null}
      <div
        className="chat-proposal-options"
        role="radiogroup"
        aria-label="Proposal options"
      >
        {options.map((option, index) => (
          <div
            className="chat-proposal-option"
            data-selected={selectedChoice === index ? 'true' : undefined}
            key={`${String(index)}-${option.label}`}
            role="presentation"
            onClick={(event) => {
              if (
                isDisabled ||
                (event.target instanceof Element &&
                  event.target.closest('a') !== null)
              ) {
                return
              }
              setSelectedChoice(index)
            }}
          >
            <input
              id={`proposal-${String(seq)}-option-${String(index)}`}
              type="radio"
              name={`proposal-${String(seq)}`}
              value={String(index)}
              checked={selectedChoice === index}
              disabled={isDisabled}
              onChange={() => {
                setSelectedChoice(index)
              }}
            />
            <div className="chat-proposal-option-content">
              <div className="chat-proposal-option-heading">
                <label
                  className="chat-proposal-option-label"
                  htmlFor={`proposal-${String(seq)}-option-${String(index)}`}
                >
                  {choiceLabel(index)} · {option.label}
                </label>
                {option.recommended ? (
                  <span className="chat-proposal-recommended">Recommended</span>
                ) : null}
              </div>
              {option.description !== null ? (
                <MarkdownMessage className="chat-proposal-option-description">
                  {option.description}
                </MarkdownMessage>
              ) : null}
              {option.pros.length > 0 || option.cons.length > 0 ? (
                <span className="chat-proposal-option-details">
                  {option.pros.map((pro) => (
                    <span className="chat-proposal-pro" key={`pro-${pro}`}>
                      + {pro}
                    </span>
                  ))}
                  {option.cons.map((con) => (
                    <span className="chat-proposal-con" key={`con-${con}`}>
                      − {con}
                    </span>
                  ))}
                </span>
              ) : null}
            </div>
          </div>
        ))}
      </div>
      {latestAnswer?.comment != null ? (
        <MarkdownMessage className="chat-proposal-answer-comment">
          {latestAnswer.comment}
        </MarkdownMessage>
      ) : (
        <>
          <label
            className="chat-proposal-comment-label"
            htmlFor={`proposal-comment-${String(seq)}`}
          >
            Comment (optional)
          </label>
          <textarea
            id={`proposal-comment-${String(seq)}`}
            className="chat-proposal-comment"
            rows={2}
            value={comment}
            disabled={isAnswered || isSubmitting}
            onPaste={(event: ClipboardEvent<HTMLTextAreaElement>) => {
              handleAttachmentPaste(event, attachmentState.onPasteFiles)
            }}
            onChange={(event) => {
              updateDraft(event.currentTarget.value)
            }}
          />
        </>
      )}
      {!isAnswered &&
      (!commentDraft.storageAvailable || !choiceDraft.storageAvailable) ? (
        <p className="conversation-error" role="status">
          Browser storage is unavailable. Drafts may not survive a page reload.
        </p>
      ) : null}
      {answer === null && !resolved ? (
        <AttachmentPreviewList
          attachments={attachmentState.attachments}
          {...(isSubmitting || isAnswered
            ? {}
            : { onRemove: attachmentState.removeAttachment })}
        />
      ) : null}
      {attachmentState.error === null ? null : (
        <p role="alert">{attachmentState.error}</p>
      )}
      <footer className="chat-proposal-footer">
        <div className="chat-proposal-actions">
          <Button
            type="button"
            variant="plain"
            className="aw-button aw-button--primary"
            disabled={
              selectedChoice === null ||
              isDisabled ||
              attachmentState.isProcessingAttachments ||
              attachmentState.hasLimitError
            }
            onClick={() => {
              void submitAnswer(selectedChoice)
            }}
          >
            Choose {selectedChoice === null ? '…' : choiceLabel(selectedChoice)}
          </Button>
          <Button
            type="button"
            variant="plain"
            className="aw-button aw-button--outline"
            disabled={
              isDisabled ||
              attachmentState.isProcessingAttachments ||
              attachmentState.hasLimitError
            }
            onClick={() => {
              void submitAnswer(null)
            }}
          >
            Ask for another option
          </Button>
        </div>
        <span className="chat-proposal-hint">
          1 / {String(options.length)} to pick · ⌘↵ to send
        </span>
      </footer>
    </article>
  )
}
