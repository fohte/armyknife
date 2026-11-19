import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { ImageLightbox } from '#components/image-lightbox'

const imageSource = 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs='

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

function render() {
  root = createRoot(container)
  act(() => {
    root?.render(<ImageLightbox src={imageSource} alt="Fixture image" />)
  })
}

async function openLightbox() {
  const trigger = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Enlarge Fixture image"]',
  )
  if (trigger === null) throw new Error('Expected the image trigger')
  await act(async () => userEvent.click(trigger))
}

it('closes the modal lightbox with Escape', async () => {
  render()
  await openLightbox()
  await act(async () => userEvent.keyboard('{Escape}'))

  expect(document.body.querySelector('dialog')).toBeNull()
})

it('closes when the image or its backdrop is clicked', async () => {
  render()
  await openLightbox()
  const image = document.body.querySelector<HTMLImageElement>(
    '.image-lightbox-dialog img',
  )
  if (image === null) throw new Error('Expected the expanded image')

  act(() => {
    image.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
  const closesOnImageClick = document.body.querySelector('dialog') === null

  await openLightbox()
  const dialog = document.body.querySelector('dialog')
  if (dialog === null) throw new Error('Expected the reopened image lightbox')
  act(() => {
    dialog.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  const getLightboxState = () => ({
    closesOnImageClick,
    closesOnBackdrop: document.body.querySelector('dialog') === null,
  })
  expect(getLightboxState()).toEqual({
    closesOnImageClick: true,
    closesOnBackdrop: true,
  })
})
