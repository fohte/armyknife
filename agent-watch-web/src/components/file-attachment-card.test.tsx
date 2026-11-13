import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { FileAttachmentCard } from '#components/file-attachment-card'

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

it('shows an inline image and expands it when selected', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <FileAttachmentCard
        message={{
          type: 'file_attachment',
          files: [
            {
              name: 'sample-dashboard.png',
              source:
                'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
              isImage: true,
              available: true,
            },
          ],
          caption: 'Review the sample dashboard.',
        }}
      />,
    )
  })

  const snapshot = () => ({
    caption: container.querySelector('.chat-file-attachment-caption')
      ?.textContent,
    images: Array.from(container.querySelectorAll('img'), (image) => ({
      alt: image.alt,
      src: image.getAttribute('src'),
    })),
    downloads: Array.from(
      container.querySelectorAll<HTMLAnchorElement>('.chat-file-download'),
      (link) => ({ text: link.textContent, href: link.getAttribute('href') }),
    ),
    dialog:
      container.querySelector('dialog')?.getAttribute('aria-label') ?? null,
  })

  expect(snapshot()).toEqual({
    caption: 'Review the sample dashboard.',
    images: [
      {
        alt: 'sample-dashboard.png',
        src: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
      },
    ],
    downloads: [],
    dialog: null,
  })

  const enlarge = container.querySelector<HTMLButtonElement>(
    'button[aria-label="Enlarge sample-dashboard.png"]',
  )
  if (enlarge === null) throw new Error('Expected an image preview button')
  act(() => {
    enlarge.click()
  })

  expect(snapshot()).toEqual({
    caption: 'Review the sample dashboard.',
    images: [
      {
        alt: 'sample-dashboard.png',
        src: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
      },
      {
        alt: 'Expanded attachment',
        src: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
      },
    ],
    downloads: [],
    dialog: 'Image preview',
  })
})

it('provides a download link for a non-image file', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <FileAttachmentCard
        message={{
          type: 'file_attachment',
          files: [
            {
              name: 'sample-report.pdf',
              source: '/api/sessions/fixture-session/files/fixture-report',
              isImage: false,
              available: true,
            },
          ],
          caption: null,
        }}
      />,
    )
  })

  expect(
    Array.from(
      container.querySelectorAll<HTMLAnchorElement>('.chat-file-download'),
      (link) => ({
        text: link.textContent,
        href: link.getAttribute('href'),
        download: link.getAttribute('download'),
      }),
    ),
  ).toEqual([
    {
      text: '↓sample-report.pdf',
      href: '/api/sessions/fixture-session/files/fixture-report',
      download: 'sample-report.pdf',
    },
  ])
})
