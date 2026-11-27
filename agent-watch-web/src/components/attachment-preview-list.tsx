import '#components/attachment-preview-list.css'

import { Button } from '@fohte/ui/button'
import type { ClipboardEvent } from 'react'

import { ImageLightbox } from '#components/image-lightbox'
import { formatFileSize } from '#lib/format-file-size'

export type AttachmentPreview = {
  id: string
  name: string
  size: number
  previewUrl: string | null
}

export function handleAttachmentPaste(
  event: ClipboardEvent<HTMLElement>,
  onPasteFiles: (files: File[]) => void,
) {
  const files = Array.from(event.clipboardData.files)
  if (files.length === 0) return
  event.preventDefault()
  onPasteFiles(files)
}

type AttachmentPreviewListProps = {
  attachments: readonly AttachmentPreview[]
  onRemove?: (id: string) => void
  label?: string
  showRemove?: boolean
}

export function AttachmentPreviewList({
  attachments,
  onRemove,
  label = 'Attachments',
  showRemove = true,
}: AttachmentPreviewListProps) {
  if (attachments.length === 0) return null

  return (
    <ul className="attachment-preview-list" aria-label={label}>
      {attachments.map((attachment) => (
        <li className="attachment-preview-item" key={attachment.id}>
          {attachment.previewUrl === null ? null : (
            <ImageLightbox src={attachment.previewUrl} alt={attachment.name} />
          )}
          <span>{attachment.name}</span>
          {attachment.previewUrl === null ? (
            <small>{formatFileSize(attachment.size)}</small>
          ) : null}
          {onRemove === undefined || !showRemove ? null : (
            <Button
              type="button"
              variant="plain"
              aria-label={`Remove ${attachment.name}`}
              onClick={() => {
                onRemove(attachment.id)
              }}
            >
              ×
            </Button>
          )}
        </li>
      ))}
    </ul>
  )
}
