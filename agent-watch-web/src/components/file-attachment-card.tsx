import '#components/file-attachment-card.css'

import { useState } from 'react'

import { formatMessageTime } from '#components/message-time'
import type { ChatItem } from '#types'

type FileAttachmentMessage = Extract<ChatItem, { type: 'file_attachment' }>

export function FileAttachmentCard({
  message,
  now,
  previewOpen = false,
}: {
  message: FileAttachmentMessage
  now?: Date
  previewOpen?: boolean
}) {
  const [unavailableImages, setUnavailableImages] = useState<Set<string>>(
    () => new Set(),
  )
  const [expandedImage, setExpandedImage] = useState<string | null>(() =>
    previewOpen
      ? (message.files.find((file) => file.isImage && file.available)?.source ??
        null)
      : null,
  )
  const time = formatMessageTime(message.timestamp, now)

  return (
    <article className="chat-file-attachment">
      <div className="chat-file-attachment-header">
        <span>Shared file</span>
        {time !== null && <time dateTime={message.timestamp}>{time}</time>}
      </div>
      {message.caption != null && message.caption !== '' && (
        <p className="chat-file-attachment-caption">{message.caption}</p>
      )}
      <ul className="chat-file-attachment-list">
        {message.files.map((file) => {
          const available =
            file.available && !unavailableImages.has(file.source)
          return (
            <li key={file.source}>
              {file.isImage && available ? (
                <button
                  className="chat-file-image-button"
                  type="button"
                  aria-label={`Enlarge ${file.name}`}
                  onClick={() => {
                    setExpandedImage(file.source)
                  }}
                >
                  <img
                    src={file.source}
                    alt={file.name}
                    loading="lazy"
                    onError={() => {
                      setUnavailableImages((current) =>
                        new Set(current).add(file.source),
                      )
                    }}
                  />
                </button>
              ) : available ? (
                <a
                  className="chat-file-download"
                  href={file.source}
                  download={file.name}
                >
                  <span aria-hidden="true">↓</span>
                  {file.name}
                </a>
              ) : (
                <span className="chat-file-unavailable">
                  {file.name} · unavailable
                </span>
              )}
            </li>
          )
        })}
      </ul>
      {expandedImage !== null && (
        <dialog
          className="chat-file-image-dialog"
          open
          aria-label="Image preview"
          onClick={() => {
            setExpandedImage(null)
          }}
        >
          <button
            type="button"
            aria-label="Close image preview"
            onClick={() => {
              setExpandedImage(null)
            }}
          >
            <img src={expandedImage} alt="Expanded attachment" />
          </button>
        </dialog>
      )}
    </article>
  )
}
