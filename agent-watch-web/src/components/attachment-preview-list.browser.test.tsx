import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { AttachmentPreviewList } from '#components/attachment-preview-list'
import { getImageLightboxState } from '#image-lightbox-state.test'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('opens a selected attachment image in the modal lightbox', async () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <AttachmentPreviewList
        attachments={[
          {
            id: 'fixture-image',
            name: 'fixture-image.png',
            size: 64,
            previewUrl:
              'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
          },
        ]}
      />,
    )
  })

  const enlarge = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Enlarge fixture-image.png"]',
  )
  if (enlarge === null) throw new Error('Expected the attachment image button')
  await act(async () => userEvent.click(enlarge))

  expect(getImageLightboxState()).toEqual({
    open: true,
    isModal: true,
    imageAlt: 'fixture-image.png',
  })
})
