import '#components/conversation-card.css'
import '#components/card-comment.css'
import '#components/question-card.css'

import { Button } from '@fohte/ui/button'
import { type KeyboardEvent, useEffect, useId, useRef, useState } from 'react'

import { numberShortcutChoice } from '#components/conversation-keyboard'
import { MarkdownMessage } from '#components/markdown-message'
import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import { QuestionSendError } from '#components/question-send-error'
import { cardDraftStorageKey, useDraft } from '#components/use-composer-draft'
import type { OpenInTerminalStatus, QuestionPrompt } from '#types'

type QuestionCardProps = {
  sessionId?: string
  requestId?: string
  questions: QuestionPrompt[]
  defaultAnswers?: Record<string, string>
  cancelled?: boolean
  canAnswer: boolean
  isMobile: boolean
  showTerminalFallback: boolean
  canOpenInTerminal: boolean
  openInTerminalStatus?: OpenInTerminalStatus
  onAnswer: (answers: Record<string, string>) => Promise<boolean>
  onSkip: () => Promise<boolean>
  onOpenInTerminal: () => void
}

function serializeSelections(selected: Record<string, string[]>) {
  const params = new URLSearchParams()
  for (const [question, choices] of Object.entries(selected)) {
    for (const choice of choices) params.append(question, choice)
  }
  return params.toString()
}

function parseSelections(serialized: string) {
  const selected: Record<string, string[]> = {}
  for (const [question, choice] of new URLSearchParams(serialized)) {
    selected[question] = [...(selected[question] ?? []), choice]
  }
  return selected
}

export function QuestionCard({
  sessionId,
  requestId,
  questions,
  defaultAnswers = {},
  cancelled = false,
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
  const [draftSessionId] = useState(() => sessionId ?? crypto.randomUUID())
  const questionCardId =
    requestId ?? questions.map((question) => question.question).join('\u0000')
  const answerDraft = useDraft(
    cardDraftStorageKey('question', draftSessionId, questionCardId, 'answers'),
    new URLSearchParams(
      Object.entries(defaultAnswers).filter(([, value]) => value !== ''),
    ).toString(),
  )
  const selectionDraft = useDraft(
    cardDraftStorageKey(
      'question',
      draftSessionId,
      questionCardId,
      'selections',
    ),
  )
  const [selected, setSelected] = useState<Record<string, string[]>>(() =>
    parseSelections(selectionDraft.draft),
  )
  const [drafts, setDrafts] = useState<Record<string, string>>(() =>
    Object.fromEntries(new URLSearchParams(answerDraft.draft).entries()),
  )
  const selectedRef = useRef(selected)
  const draftsRef = useRef(drafts)
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

  const updateAnswerDraft = (question: string, value: string) => {
    const next = { ...draftsRef.current, [question]: value }
    draftsRef.current = next
    setDrafts(next)
    answerDraft.updateDraft(
      new URLSearchParams(
        Object.entries(next).filter(([, value]) => value !== ''),
      ).toString(),
    )
  }

  const updateSelections = (
    update: (current: Record<string, string[]>) => Record<string, string[]>,
  ) => {
    const next = update(selectedRef.current)
    selectedRef.current = next
    setSelected(next)
    const serialized = serializeSelections(next)
    selectionDraft.updateDraft(serialized)
    return serialized
  }

  const sendAnswers = (
    answerValues: Record<string, string>,
    sentSelections = selectionDraft.draft,
  ) => {
    if (!canAnswer || resolved || isSending) return
    const sentDraft = answerDraft.draft
    setIsSending(true)
    void onAnswer(answerValues)
      .then((sent) => {
        setResolved(sent)
        setSendFailed(!sent)
        if (sent) {
          answerDraft.clearDraftIfUnchanged(sentDraft)
          selectionDraft.clearDraftIfUnchanged(sentSelections)
        }
      })
      .finally(() => {
        setIsSending(false)
      })
  }

  const toggleChoice = (question: QuestionPrompt, label: string) => {
    setSendFailed(false)
    updateAnswerDraft(question.question, '')
    updateSelections((current) => {
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
    updateAnswerDraft(question.question, '')
    const sentSelections = updateSelections((current) => ({
      ...current,
      [question.question]: [label],
    }))
    sendAnswers({ [question.question]: label }, sentSelections)
  }

  const submitAnswers = () => {
    if (!canSubmit) return
    sendAnswers(answers())
  }

  const skipQuestions = () => {
    if (!canAnswer || resolved || isSending) return
    const sentDraft = answerDraft.draft
    const sentSelections = selectionDraft.draft
    setIsSending(true)
    void onSkip()
      .then((skipped) => {
        setResolved(skipped)
        setSendFailed(!skipped)
        if (skipped) {
          answerDraft.clearDraftIfUnchanged(sentDraft)
          selectionDraft.clearDraftIfUnchanged(sentSelections)
        }
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
      const targetProposalCard =
        targetElement?.closest('.chat-proposal-card') ?? null
      if (targetCard === null && targetProposalCard !== null) return
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
      data-has-unanswered-question={
        !cancelled && questions.some((question) => question.answer === null)
          ? 'true'
          : undefined
      }
      data-answerable-single-choice={showNumberShortcuts ? 'true' : undefined}
      onKeyDownCapture={handleKeyDownCapture}
      ref={cardRef}
    >
      <h2>Question</h2>
      {canAnswer &&
      !resolved &&
      (!answerDraft.storageAvailable || !selectionDraft.storageAvailable) ? (
        <p className="conversation-error" role="status">
          Browser storage is unavailable. Drafts may not survive a page reload.
        </p>
      ) : null}
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
                  className="chat-card-comment"
                  value={drafts[question.question] ?? ''}
                  disabled={resolved || isSending}
                  rows={2}
                  onChange={(event) => {
                    const draft = event.currentTarget.value
                    setSendFailed(false)
                    updateSelections((current) => ({
                      ...current,
                      [question.question]: [],
                    }))
                    updateAnswerDraft(question.question, draft)
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
