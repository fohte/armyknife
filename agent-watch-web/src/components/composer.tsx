import '#components/composer.css'

import { Button } from '@fohte/ui/button'
import { ArrowUp, Plus } from 'lucide-react'
import type { ReactNode } from 'react'

import type { AttachmentPreview } from '#components/attachment-preview-list'
import { ImageLightbox } from '#components/image-lightbox'
import { formatFileSize } from '#lib/format-file-size'

export type ComposerSendState = 'on' | 'dim' | 'off'

export type ComposerProps = {
  children: ReactNode
  attachments?: readonly AttachmentPreview[]
  onRemoveAttachment?: (id: string) => void
  onAddAttachments: () => void
  sendState: ComposerSendState
  keyHints?: ReactNode
  statusText?: string
  isMobile?: boolean
  isBusy?: boolean
  trailingAction?: ReactNode
}

export function Composer({
  children,
  attachments = [],
  onRemoveAttachment,
  onAddAttachments,
  sendState,
  keyHints,
  statusText,
  isMobile = false,
  isBusy = false,
  trailingAction,
}: ComposerProps) {
  return (
    <div className="composer-shell" data-mobile={isMobile ? 'true' : undefined}>
      <div className="composer-box" aria-busy={isBusy}>
        {attachments.length > 0 ? (
          <ul className="composer-attachments" aria-label="Attachments">
            {attachments.map((attachment) => (
              <li className="composer-attachment" key={attachment.id}>
                {attachment.previewUrl === null ? (
                  <>
                    <span className="composer-attachment-name">
                      {attachment.name}
                    </span>
                    <small className="composer-attachment-size">
                      {formatFileSize(attachment.size)}
                    </small>
                  </>
                ) : (
                  <ImageLightbox
                    src={attachment.previewUrl}
                    alt={attachment.name}
                    imageClassName="composer-attachment-image"
                    triggerClassName="composer-attachment-image-trigger"
                  />
                )}
                {onRemoveAttachment === undefined ? null : (
                  <Button
                    className="composer-attachment-remove"
                    type="button"
                    variant="plain"
                    aria-label={`Remove ${attachment.name}`}
                    onClick={() => {
                      onRemoveAttachment(attachment.id)
                    }}
                  >
                    ×
                  </Button>
                )}
              </li>
            ))}
          </ul>
        ) : null}
        <div className="composer-input-slot">{children}</div>
        <div className="composer-footer">
          <Button
            className="composer-add-button"
            type="button"
            variant="plain"
            aria-label="Add attachment"
            onClick={onAddAttachments}
          >
            <Plus aria-hidden="true" size={18} />
          </Button>
          <div className="composer-key-hints">{keyHints}</div>
          <div className="composer-send-actions">
            {trailingAction}
            <Button
              className={`composer-send-button composer-send-button--${sendState}`}
              type="submit"
              variant="plain"
              aria-label="Send message"
              disabled={sendState !== 'on'}
            >
              <ArrowUp aria-hidden="true" size={18} />
            </Button>
          </div>
        </div>
      </div>
      {statusText === undefined || statusText === '' ? null : (
        <div className="composer-status" role="status">
          {statusText}
        </div>
      )}
    </div>
  )
}
