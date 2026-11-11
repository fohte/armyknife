import '#components/proposal-card.css'

import { Button } from '@fohte/ui/button'
import { useCallback, useEffect, useRef, useState } from 'react'

import { numberShortcutChoice } from '#components/conversation-keyboard'
import type { ProposalAnswer, ProposalOption } from '#types'

type ProposalCardProps = {
  seq: number
  title: string
  summary: string | null
  options: ProposalOption[]
  answer: ProposalAnswer | null
  resolved: boolean
  isMobile: boolean
  defaultChoice?: number | null
  defaultComment?: string
  onAnswer: (answer: ProposalAnswer) => Promise<boolean>
}

function choiceLabel(choice: number) {
  return String.fromCharCode(65 + choice)
}

export function ProposalCard({
  seq,
  title,
  summary,
  options,
  answer,
  resolved,
  isMobile,
  defaultChoice = null,
  defaultComment = '',
  onAnswer,
}: ProposalCardProps) {
  const [selectedChoice, setSelectedChoice] = useState<number | null>(
    answer?.choice ?? defaultChoice,
  )
  const [comment, setComment] = useState(answer?.comment ?? defaultComment)
  const [submittedAnswer, setSubmittedAnswer] = useState<ProposalAnswer | null>(
    null,
  )
  const [isSubmitting, setIsSubmitting] = useState(false)
  const cardRef = useRef<HTMLElement | null>(null)
  const latestAnswer = answer ?? submittedAnswer
  const isAnswered = latestAnswer !== null || resolved
  const isDisabled = isAnswered || isSubmitting

  useEffect(() => {
    if (answer !== null || resolved) {
      setSelectedChoice(answer?.choice ?? null)
      if (answer !== null) setComment(answer.comment ?? '')
    }
  }, [answer, resolved])

  const submitAnswer = useCallback(
    async (choice: number | null) => {
      if (isDisabled || (choice !== null && !options[choice])) return
      const response = {
        choice,
        comment: comment.trim() === '' ? null : comment.trim(),
      }
      setIsSubmitting(true)
      const succeeded = await onAnswer(response)
      setIsSubmitting(false)
      if (succeeded) setSubmittedAnswer(response)
    },
    [comment, isDisabled, onAnswer, options],
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
  }, [isDisabled, options.length, selectedChoice, submitAnswer])

  const status =
    latestAnswer !== null
      ? latestAnswer.choice === null
        ? 'asked for another option'
        : `chose ${choiceLabel(latestAnswer.choice)}`
      : resolved
        ? 'answered elsewhere'
        : 'waiting for your choice'

  return (
    <article
      className="chat-proposal-card"
      aria-label="Proposal"
      data-mobile={isMobile ? 'true' : undefined}
      data-answerable={isDisabled ? undefined : 'true'}
      ref={cardRef}
    >
      <h2>Proposal · {status}</h2>
      <h3>{title}</h3>
      {summary !== null ? (
        <p className="chat-proposal-summary">{summary}</p>
      ) : null}
      <div
        className="chat-proposal-options"
        role="radiogroup"
        aria-label="Proposal options"
      >
        {options.map((option, index) => (
          <label
            className="chat-proposal-option"
            data-selected={selectedChoice === index ? 'true' : undefined}
            key={`${String(index)}-${option.label}`}
          >
            <input
              type="radio"
              name={`proposal-${String(seq)}`}
              value={String(index)}
              checked={selectedChoice === index}
              disabled={isDisabled}
              onChange={() => {
                setSelectedChoice(index)
              }}
            />
            <span className="chat-proposal-option-content">
              <span className="chat-proposal-option-heading">
                <span className="chat-proposal-option-label">
                  {choiceLabel(index)} · {option.label}
                </span>
                {option.recommended ? (
                  <span className="chat-proposal-recommended">Recommended</span>
                ) : null}
              </span>
              {option.description !== null ? (
                <span className="chat-proposal-option-description">
                  {option.description}
                </span>
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
            </span>
          </label>
        ))}
      </div>
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
        onChange={(event) => {
          setComment(event.currentTarget.value)
        }}
      />
      <footer className="chat-proposal-footer">
        <div className="chat-proposal-actions">
          <Button
            type="button"
            variant="plain"
            className="aw-button aw-button--primary"
            disabled={selectedChoice === null || isDisabled}
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
            disabled={isDisabled}
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
