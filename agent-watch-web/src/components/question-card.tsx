import '#components/conversation-card.css'
import '#components/question-card.css'

import { Button } from '@fohte/ui/button'

import { MarkdownMessage } from '#components/markdown-message'

type QuestionCardProps = {
  question: string
  choices: string[]
  answer: string | null
  canAnswer: boolean
  isMobile: boolean
  showTerminalFallback: boolean
  canOpenInTerminal: boolean
  onAnswer: (choice: number) => void
  onOpenInTerminal: () => void
}

export function QuestionCard({
  question,
  choices,
  answer,
  canAnswer,
  isMobile,
  showTerminalFallback,
  canOpenInTerminal,
  onAnswer,
  onOpenInTerminal,
}: QuestionCardProps) {
  const selectedChoice =
    answer !== null && choices.includes(answer) ? answer : null

  return (
    <article className="chat-question-card">
      <h2>Question</h2>
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
              selectedChoice === choice
                ? 'is-selected'
                : answer !== null
                  ? 'is-unselected'
                  : undefined
            }
            aria-pressed={selectedChoice === choice}
            key={`${String(choiceIndex)}-${choice}`}
            disabled={!canAnswer || answer !== null}
            onClick={() => {
              onAnswer(choiceIndex + 1)
            }}
          >
            {!isMobile ? <kbd>{choiceIndex + 1}</kbd> : null}
            <MarkdownMessage className="chat-question-choice-text" inline>
              {choice}
            </MarkdownMessage>
            {selectedChoice === choice ? (
              <span className="chat-question-choice-check" aria-hidden="true">
                ✓
              </span>
            ) : null}
          </Button>
        ))}
      </div>
      {answer !== null && selectedChoice === null ? (
        <p className="chat-question-answer">Answer: {answer}</p>
      ) : null}
    </article>
  )
}
