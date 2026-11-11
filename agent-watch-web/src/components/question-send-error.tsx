import '#components/question-card.css'

export function QuestionSendError() {
  return (
    <p className="chat-question-send-error" role="alert">
      Your answer was not sent. Your draft is still available; select Send
      answer to retry.
    </p>
  )
}
