import '#components/peer-message.css'

import { MarkdownMessage } from '#components/markdown-message'
import { SessionLink } from '#components/session-link'
import type { ChatItem, SessionLinkTarget } from '#types'

type PeerMessageItem = Extract<ChatItem, { type: 'peer' }>

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
  const sessionName =
    foundTarget?.title ??
    (message.session_id === 'unknown'
      ? 'Unknown session'
      : message.session_title === undefined
        ? message.session_id
        : `${message.session_title} (${message.session_id})`)
  const directionLabel = message.direction === 'from' ? 'FROM' : 'TO'
  const openSession = () => {
    if (foundTarget === undefined) return
    onSelectSession(message.session_id)
  }

  return (
    <article
      className={`chat-peer-message chat-peer-message-${message.direction}`}
    >
      <div className="chat-peer-label">
        <span>{directionLabel}</span>
        {foundTarget === undefined ? (
          <span>{sessionName}</span>
        ) : (
          <SessionLink session={foundTarget} onSelectSession={onSelectSession}>
            {sessionName}
          </SessionLink>
        )}
      </div>
      <div
        className={`chat-peer-content${foundTarget === undefined ? ' chat-peer-content-unavailable' : ''}`}
        role={foundTarget === undefined ? undefined : 'link'}
        tabIndex={foundTarget === undefined ? undefined : 0}
        aria-label={
          foundTarget === undefined ? undefined : `Open ${sessionName}`
        }
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
          ) : part.type === 'image' && part.available ? (
            <img
              className="chat-peer-image"
              key={`image-${String(index)}`}
              src={part.source}
              alt="Attached image"
            />
          ) : part.type === 'image' ? (
            <p
              className="chat-peer-image-unavailable"
              key={`image-${String(index)}`}
            >
              画像はもうありません
            </p>
          ) : (
            <p className="chat-peer-file" key={`file-${String(index)}`}>
              {part.name}
            </p>
          ),
        )}
      </div>
    </article>
  )
}
