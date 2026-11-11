import '#components/conversation-card.css'
import '#components/question-card.css'

import { Button } from '@fohte/ui/button'
import {
  type KeyboardEvent,
  type SubmitEvent,
  useEffect,
  useId,
  useRef,
  useState,
} from 'react'

import { numberShortcutChoice } from '#components/conversation-keyboard'
import { MarkdownMessage } from '#components/markdown-message'
import { QuestionSendError } from '#components/question-send-error'
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
  onAnswer: (choice: number) => Promise<boolean>
  onSkip: () => Promise<boolean>
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
  const [questionSkipped, setQuestionSkipped] = useState(false)
  const [choiceAnswerAccepted, setChoiceAnswerAccepted] = useState(false)
  const [sendFailed, setSendFailed] = useState(false)
  const isSendingRef = useRef(false)
  const questionSkippedRef = useRef(false)
  const choiceAnswerAcceptedRef = useRef(false)
  const selectedChoice =
    answer !== null && choices.some((choice) => choice.label === answer)
      ? answer
      : null
  const showFreeformForm =
    (canAnswer && !questionSkipped && !choiceAnswerAccepted) ||
    (questionSkipped && draft.trim() !== '')

  const setSending = (sending: boolean) => {
    isSendingRef.current = sending
    setIsSending(sending)
  }

  const markQuestionSkipped = () => {
    questionSkippedRef.current = true
    setQuestionSkipped(true)
  }

  const markChoiceAnswerAccepted = () => {
    choiceAnswerAcceptedRef.current = true
    setChoiceAnswerAccepted(true)
  }

  const submitFreeformAnswer = (event: SubmitEvent<HTMLFormElement>) => {
    event.preventDefault()
    const answerText = draft.trim()
    if (
      (!canAnswer && !questionSkippedRef.current) ||
      choiceAnswerAcceptedRef.current ||
      isSendingRef.current ||
      answerText === ''
    ) {
      return
    }
    setSending(true)
    void (async () => {
      if (!questionSkippedRef.current) {
        const skipped = await onSkip()
        if (!skipped) return null
        markQuestionSkipped()
      }
      return onFreeformAnswer(answerText)
    })()
      .then((sent) => {
        if (sent === true) {
          setDraft('')
          setSendFailed(false)
        } else if (sent === false) {
          setSendFailed(true)
        }
      })
      .finally(() => {
        setSending(false)
      })
  }

  const skipQuestion = () => {
    if (
      !canAnswer ||
      questionSkippedRef.current ||
      choiceAnswerAcceptedRef.current ||
      isSendingRef.current
    ) {
      return
    }
    setSending(true)
    void onSkip()
      .then((skipped) => {
        if (skipped) markQuestionSkipped()
      })
      .finally(() => {
        setSending(false)
      })
  }

  const answerChoice = (choice: number) => {
    if (
      !canAnswer ||
      questionSkippedRef.current ||
      choiceAnswerAcceptedRef.current ||
      answer !== null ||
      isSendingRef.current
    ) {
      return false
    }
    setSending(true)
    void onAnswer(choice)
      .then((sent) => {
        if (sent) markChoiceAnswerAccepted()
      })
      .finally(() => {
        setSending(false)
      })
    return true
  }

  const answerChoiceRef = useRef(answerChoice)
  answerChoiceRef.current = answerChoice

  useEffect(() => {
    if (!canAnswer || isMobile || questionSkipped || choiceAnswerAccepted) {
      return
    }
    const onQuestionKeyDown = (event: WindowEventMap['keydown']) => {
      const choice = numberShortcutChoice(event, choices.length)
      if (choice === null || !answerChoiceRef.current(choice + 1)) return
      event.preventDefault()
    }

    window.addEventListener('keydown', onQuestionKeyDown)
    return () => {
      window.removeEventListener('keydown', onQuestionKeyDown)
    }
  }, [
    canAnswer,
    choiceAnswerAccepted,
    choices.length,
    isMobile,
    questionSkipped,
  ])

  const handleKeyDownCapture = (event: KeyboardEvent<HTMLElement>) => {
    if (
      event.key !== 'Escape' ||
      event.target instanceof HTMLTextAreaElement ||
      !canAnswer ||
      questionSkippedRef.current ||
      choiceAnswerAcceptedRef.current ||
      isSendingRef.current
    ) {
      return
    }
    event.preventDefault()
    event.stopPropagation()
    skipQuestion()
  }

  return (
    <article
      className="chat-question-card"
      onKeyDownCapture={handleKeyDownCapture}
    >
      <h2>
        {header === undefined || header.length === 0 ? 'Question' : header}
      </h2>
      {multiSelect ? (
        <p className="chat-question-multi-select">Multiple select question.</p>
      ) : null}
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
            disabled={
              !canAnswer ||
              questionSkipped ||
              choiceAnswerAccepted ||
              answer !== null ||
              isSending
            }
            onClick={() => {
              answerChoice(choiceIndex + 1)
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
      {showFreeformForm ? (
        <form
          className="conversation-composer chat-question-composer"
          onSubmit={(event) => {
            submitFreeformAnswer(event)
          }}
        >
          <label htmlFor={inputId}>Type something</label>
          <textarea
            id={inputId}
            value={draft}
            disabled={isSending}
            rows={2}
            onChange={(event) => {
              setDraft(event.currentTarget.value)
            }}
          />
          {sendFailed ? <QuestionSendError /> : null}
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
      ) : null}
      {canAnswer && !questionSkipped && !choiceAnswerAccepted ? (
        <div className="chat-question-actions">
          <Button
            type="button"
            variant="plain"
            className="aw-button aw-button--outline"
            disabled={isSending}
            onClick={skipQuestion}
          >
            No answer (Esc)
          </Button>
        </div>
      ) : null}
      {answer !== null && selectedChoice === null ? (
        <p className="chat-question-answer">Answer: {answer}</p>
      ) : null}
    </article>
  )
}
