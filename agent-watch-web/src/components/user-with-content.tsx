import { useEffect, useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import type { ChatItem, SessionLinkTarget, UserContentPart } from '#types'

type UserMessage = Extract<ChatItem, { type: 'user' | 'user_with_content' }>

function UserImage({
  source,
  available,
}: {
  source: string
  available: boolean
}) {
  const [isAvailable, setIsAvailable] = useState(available)

  useEffect(() => {
    setIsAvailable(available)
  }, [available, source])

  if (!isAvailable) {
    return <p className="chat-image-unavailable">画像はもうありません</p>
  }

  return (
    <img
      className="chat-user-image"
      src={source}
      alt="Attached image"
      onError={() => {
        setIsAvailable(false)
      }}
    />
  )
}

export function UserWithContent({
  message,
  onSelectSession,
  sessions,
}: {
  message: UserMessage
  onSelectSession?: (id: string) => void
  sessions?: readonly SessionLinkTarget[]
}) {
  const content: UserContentPart[] =
    message.type === 'user'
      ? [{ type: 'text', text: message.text }]
      : message.content

  return (
    <article className="chat-user-group">
      {message.from != null && message.from !== '' ? (
        <div className="chat-from">From {message.from}</div>
      ) : null}
      {content.map((part, partIndex) =>
        part.type === 'text' ? (
          <MarkdownMessage
            className="chat-user-message"
            key={`text-${String(partIndex)}`}
            onSelectSession={onSelectSession}
            sessions={sessions}
          >
            {part.text}
          </MarkdownMessage>
        ) : (
          <UserImage
            key={`image-${String(partIndex)}`}
            source={part.source}
            available={part.available}
          />
        ),
      )}
    </article>
  )
}
