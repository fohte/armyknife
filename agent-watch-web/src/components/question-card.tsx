import '#components/conversation-card.css'
import '#components/question-card.css'

import { Button } from '@fohte/ui/button'
import { type KeyboardEvent, useEffect, useId, useRef, useState } from 'react'

import { numberShortcutChoice } from '#components/conversation-keyboard'
import { MarkdownMessage } from '#components/markdown-message'
import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { QuestionSendError } from '#components/question-send-error'
import type { OpenInTerminalStatus, QuestionPrompt } from '#types'

type QuestionCardProps = {
  questions: QuestionPrompt[]
  canAnswer: boolean
  isMobile: boolean
  showTerminalFallback: boolean
  canOpenInTerminal: boolean
  openInTerminalStatus?: OpenInTerminalStatus
  onAnswer: (answers: Record<string, string>) => Promise<boolean>
  onSkip: () => Promise<boolean>
  onOpenInTerminal: () => void
}

export function QuestionCard({
  questions,
  canAnswer,
  isMobile,
  showTerminalFallback,
  canOpenInTerminal,
  openInTerminalStatus = null,
  onAnswer,
  onSkip,
  onOpenInTerminal,
}: QuestionCardProps) {
  const inputId = useId()
  const cardRef = useRef<HTMLElement | null>(null)
  const [selected, setSelected] = useState<Record<string, string[]>>({})
  const [drafts, setDrafts] = useState<Record<string, string>>({})
  const [isSending, setIsSending] = useState(false)
  const [resolved, setResolved] = useState(false)
  const [sendFailed, setSendFailed] = useState(false)
  const singleChoiceQuestion =
    questions.length === 1 && questions[0]?.multi_select === false
      ? questions[0]
      : undefined
  const showNumberShortcuts =
    !isMobile &&
    canAnswer &&
    singleChoiceQuestion !== undefined &&
    !resolved &&
    !isSending

  const answers = () =>
    Object.fromEntries(
      questions.map((question) => {
        const choices = selected[question.question] ?? []
        const draft = drafts[question.question]?.trim() ?? ''
        return [
          question.question,
          choices.length > 0
            ? question.multi_select
              ? choices.join(', ')
              : (choices[0] ?? '')
            : draft,
        ]
      }),
    )

  const canSubmit =
    canAnswer &&
    !resolved &&
    !isSending &&
    questions.every((question) => (answers()[question.question] ?? '') !== '')

  const sendAnswers = (answerValues: Record<string, string>) => {
    if (!canAnswer || resolved || isSending) return
    setIsSending(true)
    void onAnswer(answerValues)
      .then((sent) => {
        setResolved(sent)
        setSendFailed(!sent)
      })
      .finally(() => {
        setIsSending(false)
      })
  }

  const toggleChoice = (question: QuestionPrompt, label: string) => {
    setSendFailed(false)
    setDrafts((current) => ({ ...current, [question.question]: '' }))
    setSelected((current) => {
      const currentChoices = current[question.question] ?? []
      const nextChoices = question.multi_select
        ? currentChoices.includes(label)
          ? currentChoices.filter((choice) => choice !== label)
          : [...currentChoices, label]
        : [label]
      return { ...current, [question.question]: nextChoices }
    })
  }

  const answerSingleChoice = (question: QuestionPrompt, label: string) => {
    if (!canAnswer || resolved || isSending) return
    setSendFailed(false)
    setDrafts((current) => ({ ...current, [question.question]: '' }))
    setSelected((current) => ({ ...current, [question.question]: [label] }))
    sendAnswers({ [question.question]: label })
  }

  const submitAnswers = () => {
    if (!canSubmit) return
    sendAnswers(answers())
  }

  const skipQuestions = () => {
    if (!canAnswer || resolved || isSending) return
    setIsSending(true)
    void onSkip()
      .then((skipped) => {
        setResolved(skipped)
        setSendFailed(!skipped)
      })
      .finally(() => {
        setIsSending(false)
      })
  }

  useEffect(() => {
    if (!showNumberShortcuts) return
    const onQuestionKeyDown = (event: WindowEventMap['keydown']) => {
      const targetElement =
        event.target instanceof HTMLElement ? event.target : null
      const targetCard = targetElement?.closest('.chat-question-card') ?? null
      if (targetCard !== null) {
        if (targetCard !== cardRef.current) return
      } else {
        const answerableCards = document.querySelectorAll(
          '.chat-question-card[data-answerable-single-choice="true"]',
        )
        if (
          answerableCards.item(answerableCards.length - 1) !== cardRef.current
        ) {
          return
        }
      }

      const choice = numberShortcutChoice(
        event,
        singleChoiceQuestion.choices.length,
      )
      if (choice === null) return
      const selectedChoice = singleChoiceQuestion.choices[choice]
      if (selectedChoice === undefined) return
      event.preventDefault()
      answerSingleChoice(singleChoiceQuestion, selectedChoice.label)
    }

    window.addEventListener('keydown', onQuestionKeyDown)
    return () => {
      window.removeEventListener('keydown', onQuestionKeyDown)
    }
  }, [answerSingleChoice, showNumberShortcuts, singleChoiceQuestion])

  const handleKeyDownCapture = (event: KeyboardEvent<HTMLElement>) => {
    if (
      event.key !== 'Escape' ||
      event.target instanceof HTMLTextAreaElement ||
      !canAnswer ||
      resolved ||
      isSending
    ) {
      return
    }
    event.preventDefault()
    event.stopPropagation()
    skipQuestions()
  }

  return (
    <article
      className="chat-question-card"
      data-answerable-single-choice={showNumberShortcuts ? 'true' : undefined}
      onKeyDownCapture={handleKeyDownCapture}
      ref={cardRef}
    >
      <h2>Question</h2>
      {showTerminalFallback ? (
        <div className="conversation-terminal-fallback">
          <span>Answer in terminal to continue.</span>
          {canOpenInTerminal ? (
            <OpenInTerminalAction
              status={openInTerminalStatus}
              onClick={onOpenInTerminal}
              announce={false}
            />
          ) : null}
        </div>
      ) : null}
      {questions.map((question, questionIndex) => {
        const selectedChoices = selected[question.question] ?? []
        const hasRecordedAnswer = question.answer !== null
        const recordedChoices =
          question.answer === null
            ? []
            : question.answer
                .split(', ')
                .filter((label) =>
                  question.choices.some((choice) => choice.label === label),
                )
        const selectedLabels =
          selectedChoices.length > 0 ? selectedChoices : recordedChoices
        const draftId = `${inputId}-${String(questionIndex)}`

        return (
          <section
            className="chat-question-prompt"
            key={`${String(questionIndex)}-${question.question}`}
          >
            <h3>
              {question.header === undefined || question.header.length === 0
                ? `Question ${String(questionIndex + 1)}`
                : question.header}
            </h3>
            <MarkdownMessage className="chat-question-text">
              {question.question}
            </MarkdownMessage>
            {question.multi_select ? (
              <p className="chat-question-multi-select">
                Select one or more choices.
              </p>
            ) : null}
            <div className="chat-question-choices">
              {question.choices.map((choice, choiceIndex) => {
                const isSelected = selectedLabels.includes(choice.label)
                return (
                  <Button
                    type="button"
                    variant="plain"
                    className={
                      isSelected
                        ? 'is-selected'
                        : hasRecordedAnswer
                          ? 'is-unselected'
                          : undefined
                    }
                    aria-pressed={isSelected}
                    key={`${String(choiceIndex)}-${choice.label}`}
                    disabled={!canAnswer || resolved || isSending}
                    onClick={() => {
                      if (singleChoiceQuestion !== undefined) {
                        answerSingleChoice(question, choice.label)
                      } else {
                        toggleChoice(question, choice.label)
                      }
                    }}
                  >
                    {showNumberShortcuts && choiceIndex < 9 ? (
                      <kbd>{choiceIndex + 1}</kbd>
                    ) : null}
                    <span className="chat-question-choice-content">
                      <MarkdownMessage
                        className="chat-question-choice-text"
                        inline
                      >
                        {choice.label}
                      </MarkdownMessage>
                      {choice.description !== undefined &&
                      choice.description.length > 0 ? (
                        <small className="chat-question-choice-description">
                          <MarkdownMessage inline>
                            {choice.description}
                          </MarkdownMessage>
                        </small>
                      ) : null}
                    </span>
                    {isSelected ? (
                      <span
                        className="chat-question-choice-check"
                        aria-hidden="true"
                      >
                        ✓
                      </span>
                    ) : null}
                  </Button>
                )
              })}
            </div>
            {canAnswer ? (
              <div className="chat-question-composer">
                <label htmlFor={draftId}>Or enter your own answer</label>
                <textarea
                  id={draftId}
                  value={drafts[question.question] ?? ''}
                  disabled={resolved || isSending}
                  rows={2}
                  onChange={(event) => {
                    const draft = event.currentTarget.value
                    setSendFailed(false)
                    setSelected((current) => ({
                      ...current,
                      [question.question]: [],
                    }))
                    setDrafts((current) => ({
                      ...current,
                      [question.question]: draft,
                    }))
                  }}
                />
              </div>
            ) : null}
            {hasRecordedAnswer && selectedLabels.length === 0 ? (
              <p className="chat-question-answer">Answer: {question.answer}</p>
            ) : null}
          </section>
        )
      })}
      {sendFailed ? <QuestionSendError /> : null}
      {canAnswer && !resolved ? (
        <div className="chat-question-actions">
          <Button
            type="button"
            variant="plain"
            className="aw-button aw-button--outline"
            disabled={isSending}
            onClick={skipQuestions}
          >
            No answer (Esc)
          </Button>
          <Button
            type="button"
            variant="plain"
            className="aw-button aw-button--primary"
            disabled={!canSubmit}
            onClick={submitAnswers}
          >
            Send answers
          </Button>
        </div>
      ) : null}
    </article>
  )
}
