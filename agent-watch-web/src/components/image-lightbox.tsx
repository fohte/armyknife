import '#components/image-lightbox.css'

import { type SyntheticEvent, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'

export function ImageLightbox({
  src,
  alt,
  title,
  imageClassName,
  triggerClassName,
  loading,
  decoding,
  onError,
  defaultOpen = false,
}: {
  src: string
  alt: string
  title?: string
  imageClassName?: string
  triggerClassName?: string
  loading?: 'eager' | 'lazy'
  decoding?: 'async' | 'auto' | 'sync'
  onError?: (event: SyntheticEvent<HTMLImageElement>) => void
  defaultOpen?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  const dialogRef = useRef<HTMLDialogElement>(null)

  useEffect(() => {
    const dialog = dialogRef.current
    if (dialog === null) return

    if (open && !dialog.open) dialog.showModal()
  }, [open])

  const close = () => {
    dialogRef.current?.close()
    setOpen(false)
  }

  return (
    <>
      <button
        className={`image-lightbox-trigger${triggerClassName === undefined ? '' : ` ${triggerClassName}`}`}
        type="button"
        aria-label={`Enlarge ${alt === '' ? 'image' : alt}`}
        onClick={(event) => {
          event.stopPropagation()
          setOpen(true)
        }}
      >
        <img
          className={imageClassName}
          src={src}
          alt={alt}
          title={title}
          loading={loading}
          decoding={decoding}
          onError={onError}
        />
      </button>
      {open &&
        createPortal(
          <dialog
            aria-label="Image preview"
            className="image-lightbox-dialog"
            onCancel={(event) => {
              event.preventDefault()
              close()
            }}
            onClick={(event) => {
              event.stopPropagation()
              if (event.target === event.currentTarget) close()
            }}
            ref={dialogRef}
          >
            <button
              className="image-lightbox-close"
              type="button"
              aria-label="Close image preview"
              onClick={close}
            >
              <img src={src} alt={alt} />
            </button>
          </dialog>,
          document.body,
        )}
    </>
  )
}
