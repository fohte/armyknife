import '#components/file-attachment-card.css'

import { useEffect, useState } from 'react'

import { requestUtf8Text } from '#components/conversation-requests'
import {
  type AttachmentPreviewFile,
  FileContentPreview,
} from '#components/file-content-preview'
import { formatMessageTime } from '#components/message-time'
import type { ChatItem } from '#types'

type FileAttachmentMessage = Extract<ChatItem, { type: 'file_attachment' }>

function useTextPreview(file: AttachmentPreviewFile) {
  const [preview, setPreview] = useState<
    | { status: 'loading' }
    | { status: 'unavailable' }
    | { status: 'loaded'; content: string }
  >({ status: 'loading' })

  useEffect(() => {
    let active = true

    void requestUtf8Text(file.source).match(
      (content) => {
        if (active) setPreview({ status: 'loaded', content })
      },
      () => {
        if (active) setPreview({ status: 'unavailable' })
      },
    )

    return () => {
      active = false
    }
  }, [file.source])

  return preview
}

function TextFilePreview({ file }: { file: AttachmentPreviewFile }) {
  const preview = useTextPreview(file)
  return (
    <FileContentPreview
      file={file}
      loading={preview.status === 'loading'}
      {...(preview.status === 'loaded' ? { textContent: preview.content } : {})}
    />
  )
}

function AttachmentPreview({
  file,
  onEnlargeImage,
}: {
  file: AttachmentPreviewFile
  onEnlargeImage: (source: string) => void
}) {
  if (!file.available) {
    return <FileContentPreview file={file} />
  }

  if (file.previewType === 'text' || file.previewType === 'markdown') {
    return <TextFilePreview file={file} />
  }

  return <FileContentPreview file={file} onEnlargeImage={onEnlargeImage} />
}

export function FileAttachmentCard({
  message,
  now,
  previewOpen = false,
}: {
  message: FileAttachmentMessage
  now?: Date
  previewOpen?: boolean
}) {
  const [expandedImage, setExpandedImage] = useState<string | null>(() =>
    previewOpen
      ? (message.files.find(
          (file) => file.previewType === 'image' && file.available,
        )?.source ?? null)
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
        {message.files.map((file) => (
          <li className="chat-file-attachment-item" key={file.source}>
            <AttachmentPreview file={file} onEnlargeImage={setExpandedImage} />
          </li>
        ))}
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
