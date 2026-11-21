import '#components/attachment-preview-list.css'

import { Button } from '@fohte/ui/button'
import type { ClipboardEvent, DragEvent as ReactDragEvent } from 'react'

import { containsFiles, getDroppedFiles } from '#components/file-drop'
import { ImageLightbox } from '#components/image-lightbox'

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

export function handleAttachmentDrag(event: ReactDragEvent<HTMLElement>) {
  if (!containsFiles(event.dataTransfer)) return
  event.preventDefault()
  event.stopPropagation()
  event.dataTransfer.dropEffect = 'copy'
}

export function handleAttachmentDragLeave(event: ReactDragEvent<HTMLElement>) {
  if (!containsFiles(event.dataTransfer)) return
  event.stopPropagation()
}

export function handleAttachmentDrop(
  event: ReactDragEvent<HTMLElement>,
  onPasteFiles: (files: File[]) => void,
) {
  if (!containsFiles(event.dataTransfer)) return
  event.preventDefault()
  event.stopPropagation()
  const files = getDroppedFiles(event.dataTransfer)
  if (files.length > 0) onPasteFiles(files)
}

type AttachmentPreviewListProps = {
  attachments: readonly AttachmentPreview[]
  onRemove?: (id: string) => void
  label?: string
  showRemove?: boolean
}

function formatFileSize(bytes: number) {
  if (bytes < 1024) return `${String(bytes)} B`
  const kibibytes = bytes / 1024
  const formatted = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
  })
  if (kibibytes < 1024) return `${formatted.format(kibibytes)} KiB`
  return `${formatted.format(kibibytes / 1024)} MiB`
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
