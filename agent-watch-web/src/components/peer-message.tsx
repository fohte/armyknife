import '#components/peer-message.css'

import { MarkdownMessage } from '#components/markdown-message'
import { SessionLink } from '#components/session-link'
import type { ChatItem, SessionLinkTarget } from '#types'

type PeerMessageItem = Extract<ChatItem, { type: 'peer' }>

function sessionTarget(
  sessionId: string,
  sessions: readonly SessionLinkTarget[],
): SessionLinkTarget {
  return (
    sessions.find((session) => session.id === sessionId) ?? {
      id: sessionId,
      category: 'done',
      isPaused: false,
      task: null,
      repo: 'unknown',
    }
  )
}

export function PeerMessage({
  message,
  onSelectSession,
  sessions,
}: {
  message: PeerMessageItem
  onSelectSession: (id: string) => void
  sessions: readonly SessionLinkTarget[]
}) {
  const foundTarget = sessions.find(
    (session) => session.id === message.session_id,
  )
  const target = foundTarget ?? sessionTarget(message.session_id, sessions)
  const sessionName = target.title ?? message.session_title ?? target.id
  const directionLabel = message.direction === 'from' ? 'FROM' : 'TO'
  const openSession = () => {
    onSelectSession(message.session_id)
  }

  return (
    <article
      className={`chat-peer-message chat-peer-message-${message.direction}`}
    >
      <div className="chat-peer-label">
        <span>{directionLabel}</span>
        <SessionLink session={target} onSelectSession={onSelectSession}>
          {sessionName}
        </SessionLink>
      </div>
      <div
        className="chat-peer-content"
        role="link"
        tabIndex={0}
        aria-label={`Open ${sessionName}`}
        onClick={(event) => {
          if (
            event.target instanceof Element &&
            event.target.closest('a') !== null
          ) {
            return
          }
          openSession()
        }}
        onKeyDown={(event) => {
          if (
            event.target instanceof Element &&
            event.target.closest('a') !== null
          ) {
            return
          }
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            openSession()
          }
        }}
      >
        {message.content.map((part, index) =>
          part.type === 'text' ? (
            <MarkdownMessage
              className="chat-peer-text"
              key={`text-${String(index)}`}
              onSelectSession={onSelectSession}
              sessions={sessions}
            >
              {part.text}
            </MarkdownMessage>
          ) : part.available ? (
            <img
              className="chat-peer-image"
              key={`image-${String(index)}`}
              src={part.source}
              alt="Attached image"
            />
          ) : (
            <p
              className="chat-peer-image-unavailable"
              key={`image-${String(index)}`}
            >
              画像はもうありません
            </p>
          ),
        )}
      </div>
    </article>
  )
}
