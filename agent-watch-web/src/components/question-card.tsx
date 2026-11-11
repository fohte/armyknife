import { Button } from '@fohte/ui/button'
import { type SubmitEvent, useId, useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import type { QuestionChoice } from '#types'

type QuestionCardProps = {
  header?: string
  question: string
  choices: QuestionChoice[]
  multiSelect: boolean
  answer: string | null
  canAnswer: boolean
  isMobile: boolean
  showTerminalFallback: boolean
  canOpenInTerminal: boolean
  onAnswer: (choice: number) => void
  onSkip: () => void
  onFreeformAnswer: (answer: string) => Promise<boolean>
  onOpenInTerminal: () => void
}

export function QuestionCard({
  header,
  question,
  choices,
  multiSelect,
  answer,
  canAnswer,
  isMobile,
  showTerminalFallback,
  canOpenInTerminal,
  onAnswer,
  onSkip,
  onFreeformAnswer,
  onOpenInTerminal,
}: QuestionCardProps) {
  const inputId = useId()
  const [draft, setDraft] = useState('')
  const [isSending, setIsSending] = useState(false)
  const selectedChoice =
    answer !== null && choices.some((choice) => choice.label === answer)
      ? answer
      : null

  const submitFreeformAnswer = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const answerText = draft.trim()
    if (!canAnswer || isSending || answerText === '') return
    setIsSending(true)
    void onFreeformAnswer(answerText)
      .then((sent) => {
        if (sent) setDraft('')
      })
      .finally(() => {
        setIsSending(false)
      })
  }

  return (
    <article className="chat-question-card">
      <h2>
        {header === undefined || header.length === 0 ? 'Question' : header}
      </h2>
      {multiSelect ? <p>Select all that apply.</p> : null}
      <MarkdownMessage className="chat-question-text">
        {question}
      </MarkdownMessage>
      {showTerminalFallback ? (
        <div className="conversation-terminal-fallback">
          <span>Answer in terminal to continue.</span>
          {canOpenInTerminal ? (
            <Button type="button" variant="plain" onClick={onOpenInTerminal}>
              Open in terminal
            </Button>
          ) : null}
        </div>
      ) : null}
      <div className="chat-question-choices">
        {choices.map((choice, choiceIndex) => (
          <Button
            type="button"
            variant="plain"
            className={
              selectedChoice === choice.label
                ? 'is-selected'
                : answer !== null
                  ? 'is-unselected'
                  : undefined
            }
            aria-pressed={selectedChoice === choice.label}
            key={`${String(choiceIndex)}-${choice.label}`}
            disabled={!canAnswer || answer !== null}
            onClick={() => {
              onAnswer(choiceIndex + 1)
            }}
          >
            {!isMobile ? <kbd>{choiceIndex + 1}</kbd> : null}
            <span className="chat-question-choice-content">
              <MarkdownMessage className="chat-question-choice-text" inline>
                {choice.label}
              </MarkdownMessage>
              {choice.description !== undefined &&
              choice.description.length > 0 ? (
                <small className="chat-question-choice-description">
                  <MarkdownMessage inline>{choice.description}</MarkdownMessage>
                </small>
              ) : null}
            </span>
            {selectedChoice === choice.label ? (
              <span className="chat-question-choice-check" aria-hidden="true">
                ✓
              </span>
            ) : null}
          </Button>
        ))}
      </div>
      {canAnswer ? (
        <>
          <form
            className="conversation-composer chat-question-composer"
            onSubmit={(event) => {
              submitFreeformAnswer(event)
            }}
          >
            <label htmlFor={inputId}>Chat about this</label>
            <textarea
              id={inputId}
              placeholder="Type something."
              value={draft}
              disabled={isSending}
              rows={2}
              onChange={(event) => {
                setDraft(event.currentTarget.value)
              }}
            />
            <div className="chat-question-actions">
              <Button
                type="submit"
                variant="plain"
                className="aw-button aw-button--primary"
                disabled={isSending || draft.trim() === ''}
              >
                Send answer
              </Button>
            </div>
          </form>
          <div className="chat-question-actions">
            <Button
              type="button"
              variant="plain"
              className="aw-button aw-button--outline"
              onClick={() => {
                onSkip()
              }}
            >
              No answer (Esc)
            </Button>
          </div>
        </>
      ) : null}
      {answer !== null && selectedChoice === null ? (
        <p className="chat-question-answer">Answer: {answer}</p>
      ) : null}
    </article>
  )
}
