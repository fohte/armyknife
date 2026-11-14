import '#components/user-with-content.css'

import { useEffect, useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import { formatMessageTime } from '#components/message-time'
import type {
  ChatItem,
  RewindMode,
  Session,
  SessionLinkTarget,
  UserContentPart,
} from '#types'

type UserMessage = Extract<
  ChatItem,
  { type: 'user' | 'queued_user' | 'user_with_content' }
>

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
      loading="lazy"
      decoding="async"
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
  now,
  canRewind,
  sessionEngine,
  onRewind = () => Promise.resolve(false),
  rewindMenuOpen = false,
}: {
  message: UserMessage
  onSelectSession?: (id: string) => void
  sessions?: readonly SessionLinkTarget[]
  now?: Date
  canRewind?: boolean
  sessionEngine?: Session['engine']
  onRewind?: (
    messageId: string,
    mode: RewindMode,
    draft: string,
  ) => Promise<boolean>
  rewindMenuOpen?: boolean
}) {
  const content: UserContentPart[] =
    message.type === 'user'
      ? [{ type: 'text', text: message.text }]
      : message.content
  const queued = message.type === 'queued_user'
  const isOwnMessage = queued || message.from == null || message.from === ''
  const time = formatMessageTime(message.timestamp, now)
  const label = queued
    ? 'You · Queued'
    : message.from != null && message.from !== ''
      ? `From ${message.from}`
      : 'You'
  const rewindId =
    message.type === 'user' || message.type === 'user_with_content'
      ? message.rewindId
      : undefined
  const draftText = content
    .filter(
      (part): part is Extract<UserContentPart, { type: 'text' }> =>
        part.type === 'text',
    )
    .map((part) => part.text)
    .join('\n')

  return (
    <article
      className="chat-user-group"
      data-own-message={isOwnMessage ? 'true' : undefined}
    >
      <div className="chat-from">
        {label}
        {time !== null && (
          <>
            {' · '}
            <time dateTime={message.timestamp}>{time}</time>
          </>
        )}
        {canRewind === true && rewindId !== undefined ? (
          <details className="chat-rewind-menu" open={rewindMenuOpen}>
            <summary>Rewind</summary>
            <div
              className="chat-rewind-menu-options"
              role="group"
              aria-label="Rewind options"
            >
              {sessionEngine === 'codex' ? (
                <button
                  type="button"
                  onClick={() => {
                    void onRewind(rewindId, 'conversation', draftText)
                  }}
                >
                  Conversation only
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => {
                      void onRewind(
                        rewindId,
                        'conversation_and_code',
                        draftText,
                      )
                    }}
                  >
                    Conversation and code
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      void onRewind(rewindId, 'conversation', draftText)
                    }}
                  >
                    Conversation only
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      void onRewind(rewindId, 'code', draftText)
                    }}
                  >
                    Code only
                  </button>
                </>
              )}
            </div>
          </details>
        ) : null}
      </div>
      {content.map((part, partIndex) =>
        part.type === 'text' ? (
          <MarkdownMessage
            className="chat-user-message"
            key={`text-${String(partIndex)}`}
            {...(onSelectSession === undefined ? {} : { onSelectSession })}
            {...(sessions === undefined ? {} : { sessions })}
          >
            {part.text}
          </MarkdownMessage>
        ) : part.type === 'image' ? (
          <UserImage
            key={`image-${String(partIndex)}`}
            source={part.source}
            available={part.available}
          />
        ) : (
          <p className="chat-user-file" key={`file-${String(partIndex)}`}>
            {part.name}
          </p>
        ),
      )}
    </article>
  )
}
