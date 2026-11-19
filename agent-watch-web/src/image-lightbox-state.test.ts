export function getImageLightboxState() {
  const dialog = document.body.querySelector<HTMLDialogElement>(
    '.image-lightbox-dialog',
  )

  return {
    open: dialog?.open ?? false,
    isModal: dialog?.matches(':modal') ?? false,
    imageAlt: dialog?.querySelector('img')?.alt ?? null,
  }
}
