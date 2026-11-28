import '#components/withdrawn-card.css'

import { formatMessageTime } from '#components/message-time'

type WithdrawnCardProps = {
  kind: 'To-do' | 'Proposal'
  title: string
  cardId?: string
  sessionEndedAt?: string
  now?: Date
}

export function WithdrawnCard({
  kind,
  title,
  cardId,
  sessionEndedAt,
  now,
}: WithdrawnCardProps) {
  const time = formatMessageTime(sessionEndedAt, now)
  return (
    <article
      className="chat-withdrawn-card"
      data-resolution="withdrawn"
      data-conversation-card-id={cardId}
      aria-label={
        kind === 'To-do' ? 'User task, withdrawn' : 'Proposal, withdrawn'
      }
    >
      <h2>
        {kind} · withdrawn
        {time !== null && (
          <span className="chat-withdrawn-card-note">
            {' · session ended '}
            <time dateTime={sessionEndedAt}>{time}</time>
          </span>
        )}
      </h2>
      <h3>{title}</h3>
    </article>
  )
}
