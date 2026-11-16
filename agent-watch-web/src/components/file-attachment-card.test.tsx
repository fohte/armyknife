import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { FileAttachmentCard } from '#components/file-attachment-card'
import type { ChatItem } from '#types'

type FileAttachmentMessage = Extract<ChatItem, { type: 'file_attachment' }>
type AttachmentFile = FileAttachmentMessage['files'][number]

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

function attachmentFile(
  name: string,
  previewType: AttachmentFile['previewType'],
  overrides: Partial<AttachmentFile> = {},
): AttachmentFile {
  return {
    name,
    source: `/api/sessions/fixture-session/files/${name}`,
    size: 128,
    isImage: previewType === 'image',
    previewType,
    available: true,
    ...overrides,
  }
}

function renderCard(file: AttachmentFile, caption: string | null = null) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <FileAttachmentCard
        message={{ type: 'file_attachment', files: [file], caption }}
      />,
    )
  })
}

async function renderFetchedCard(
  file: AttachmentFile,
  content: string | Uint8Array,
  contentType: string,
  openDeferred = false,
): Promise<{
  initialFetchCalls: string[]
  initialDetailsOpen: boolean | null
  initialCodeBlockCount: number
  fetchCalls: string[]
}> {
  const responseBody =
    typeof content === 'string' ? content : new Uint8Array(content).buffer
  const response = new Response(responseBody, {
    headers: { 'Content-Type': contentType },
  })
  const fetcher = vi.fn<(input: RequestInfo | URL) => Promise<Response>>(() =>
    Promise.resolve(response),
  )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <FileAttachmentCard
        message={{ type: 'file_attachment', files: [file], caption: null }}
      />,
    )
  })
  const fetchCalls = () =>
    fetcher.mock.calls.map(([input]) =>
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.href
          : input.url,
    )
  const initialFetchCalls = fetchCalls()
  const initialDetailsOpen =
    container.querySelector<HTMLDetailsElement>('.chat-file-collapsed-preview')
      ?.open ?? null
  const initialCodeBlockCount = container.querySelectorAll(
    '.markdown-message, .syntax-code-block',
  ).length
  if (openDeferred) {
    const summary = container
      .querySelector<HTMLDetailsElement>('.chat-file-collapsed-preview')
      ?.querySelector('summary')
    if (summary === null || summary === undefined) {
      throw new Error('Expected a deferred preview summary')
    }
    await act(async () => {
      summary.click()
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  } else {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0))
    })
  }
  return {
    initialFetchCalls,
    initialDetailsOpen,
    initialCodeBlockCount,
    fetchCalls: fetchCalls(),
  }
}

async function renderWithFailedFetch(file: AttachmentFile): Promise<string[]> {
  const fetcher = vi.fn<(input: RequestInfo | URL) => Promise<Response>>(() =>
    Promise.reject(new Error('Example request failure')),
  )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <FileAttachmentCard
        message={{ type: 'file_attachment', files: [file], caption: null }}
      />,
    )
  })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  return fetcher.mock.calls.map(([input]) =>
    typeof input === 'string'
      ? input
      : input instanceof URL
        ? input.href
        : input.url,
  )
}

const downloadSnapshot = () =>
  Array.from(
    container.querySelectorAll<HTMLAnchorElement>('.chat-file-download'),
    (link) => ({
      text: link.textContent,
      href: link.getAttribute('href'),
      download: link.getAttribute('download'),
      label: link.getAttribute('aria-label'),
    }),
  )

it('shows an inline image, a download link, and an enlarged preview', () => {
  const file = attachmentFile('sample-dashboard.png', 'image', {
    source: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
  })
  renderCard(file, 'Review the sample dashboard.')

  const snapshot = () => ({
    caption: container.querySelector('.chat-file-attachment-caption')
      ?.textContent,
    images: Array.from(container.querySelectorAll('img'), (image) => ({
      alt: image.alt,
      src: image.getAttribute('src'),
    })),
    downloads: downloadSnapshot(),
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
    downloads: [
      {
        text: '↓sample-dashboard.png',
        href: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
        download: 'sample-dashboard.png',
        label: 'Download sample-dashboard.png',
      },
    ],
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
    downloads: [
      {
        text: '↓sample-dashboard.png',
        href: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
        download: 'sample-dashboard.png',
        label: 'Download sample-dashboard.png',
      },
    ],
    dialog: 'Image preview',
  })
})

it('renders Markdown attachment text with the conversation Markdown renderer', async () => {
  const file = attachmentFile('sample-notes.md', 'markdown')
  const { initialFetchCalls, fetchCalls } = await renderFetchedCard(
    file,
    '# Sample notes\n\nA **bold** note.',
    'text/markdown; charset=utf-8',
  )

  const snapshot = () => ({
    initialFetchCalls,
    fetchCalls,
    headings: Array.from(container.querySelectorAll('h1'), (node) =>
      node.textContent.trim(),
    ),
    bold: Array.from(container.querySelectorAll('strong'), (node) =>
      node.textContent.trim(),
    ),
    preformattedBlocks: container.querySelectorAll('pre').length,
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    initialFetchCalls: [file.source],
    fetchCalls: [file.source],
    headings: ['Sample notes'],
    bold: ['bold'],
    preformattedBlocks: 0,
    downloads: [
      {
        text: '↓sample-notes.md',
        href: file.source,
        download: 'sample-notes.md',
        label: 'Download sample-notes.md',
      },
    ],
  })
})

it('renders non-Markdown text as a syntax code block', async () => {
  const file = attachmentFile('sample-config.json', 'text')
  const { initialFetchCalls, fetchCalls } = await renderFetchedCard(
    file,
    '{"mode":"sample","ready":true}',
    'application/json; charset=utf-8',
  )

  const snapshot = () => ({
    initialFetchCalls,
    fetchCalls,
    codeBlocks: Array.from(
      container.querySelectorAll<HTMLPreElement>('.syntax-code-block'),
      (block) => ({
        language: block.getAttribute('data-language'),
        text: block.textContent,
      }),
    ),
    markdown: container.querySelector('.markdown-message') !== null,
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    initialFetchCalls: [file.source],
    fetchCalls: [file.source],
    codeBlocks: [
      {
        language: 'json',
        text: '{"mode":"sample","ready":true}',
      },
    ],
    markdown: false,
    downloads: [
      {
        text: '↓sample-config.json',
        href: file.source,
        download: 'sample-config.json',
        label: 'Download sample-config.json',
      },
    ],
  })
})

it('keeps a long text preview collapsed until the reader opens it', async () => {
  const file = attachmentFile('sample-output.log', 'text', { size: 3600 })
  const content = 'Example output line.\n'.repeat(180)
  const {
    initialFetchCalls,
    initialDetailsOpen,
    initialCodeBlockCount,
    fetchCalls,
  } = await renderFetchedCard(file, content, 'text/plain; charset=utf-8', true)

  const details = container.querySelector<HTMLDetailsElement>(
    '.chat-file-collapsed-preview',
  )
  const snapshot = () => ({
    initialFetchCalls,
    initialDetailsOpen,
    initialCodeBlockCount,
    fetchCalls,
    expanded: details?.open,
    summary: details?.querySelector('summary')?.textContent,
    codeText: details?.querySelector('pre')?.textContent,
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    initialFetchCalls: [],
    initialDetailsOpen: false,
    initialCodeBlockCount: 0,
    fetchCalls: [file.source],
    expanded: true,
    summary: 'Show file contents',
    codeText: content,
    downloads: [
      {
        text: '↓sample-output.log',
        href: file.source,
        download: 'sample-output.log',
        label: 'Download sample-output.log',
      },
    ],
  })
})

it('does not fetch a large Markdown preview before it is opened', async () => {
  const file = attachmentFile('sample-guide.md', 'markdown', { size: 3600 })
  const content = '# Sample guide\n\nThe **example** is ready.'
  const {
    initialFetchCalls,
    initialDetailsOpen,
    initialCodeBlockCount,
    fetchCalls,
  } = await renderFetchedCard(
    file,
    content,
    'text/markdown; charset=utf-8',
    true,
  )

  const details = container.querySelector<HTMLDetailsElement>(
    '.chat-file-collapsed-preview',
  )
  const snapshot = () => ({
    initialFetchCalls,
    initialDetailsOpen,
    initialCodeBlockCount,
    fetchCalls,
    expanded: details?.open,
    headings: Array.from(container.querySelectorAll('h1'), (node) =>
      node.textContent.trim(),
    ),
    emphasis: Array.from(container.querySelectorAll('strong'), (node) =>
      node.textContent.trim(),
    ),
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    initialFetchCalls: [],
    initialDetailsOpen: false,
    initialCodeBlockCount: 0,
    fetchCalls: [file.source],
    expanded: true,
    headings: ['Sample guide'],
    emphasis: ['example'],
    downloads: [
      {
        text: '↓sample-guide.md',
        href: file.source,
        download: 'sample-guide.md',
        label: 'Download sample-guide.md',
      },
    ],
  })
})

it.each([
  {
    name: 'a video',
    filename: 'sample-clip.mp4',
    previewType: 'video' as const,
    tag: 'VIDEO',
  },
  {
    name: 'an audio file',
    filename: 'sample-audio.mp3',
    previewType: 'audio' as const,
    tag: 'AUDIO',
  },
])(
  '$name has an inline player and a download link',
  ({ filename, previewType, tag }) => {
    const file = attachmentFile(filename, previewType, {
      source:
        previewType === 'video'
          ? 'data:video/mp4;base64,AA=='
          : 'data:audio/mpeg;base64,AA==',
    })
    renderCard(file)

    const snapshot = () => ({
      player: container.querySelector(tag.toLowerCase())?.tagName ?? null,
      controls:
        container.querySelector(tag.toLowerCase())?.hasAttribute('controls') ??
        false,
      source:
        container.querySelector(tag.toLowerCase())?.getAttribute('src') ?? null,
      preload:
        container.querySelector(tag.toLowerCase())?.getAttribute('preload') ??
        null,
      downloads: downloadSnapshot(),
    })
    expect(snapshot()).toEqual({
      player: tag,
      controls: true,
      source: file.source,
      preload: 'metadata',
      downloads: [
        {
          text: `↓${filename}`,
          href: file.source,
          download: filename,
          label: `Download ${filename}`,
        },
      ],
    })
  },
)

it('opens PDF files in the browser and keeps a separate download link', () => {
  const file = attachmentFile('sample-report.pdf', 'pdf')
  renderCard(file)

  const snapshot = () => ({
    openLink: Array.from(
      container.querySelectorAll<HTMLAnchorElement>('.chat-file-open-pdf'),
      (link) => ({
        text: link.textContent,
        href: link.getAttribute('href'),
        target: link.getAttribute('target'),
        rel: link.getAttribute('rel'),
      }),
    ),
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    openLink: [
      {
        text: 'Open PDF',
        href: file.source,
        target: '_blank',
        rel: 'noopener noreferrer',
      },
    ],
    downloads: [
      {
        text: '↓sample-report.pdf',
        href: file.source,
        download: 'sample-report.pdf',
        label: 'Download sample-report.pdf',
      },
    ],
  })
})

it('does not fetch or render HTML attachments', () => {
  const file = attachmentFile('sample-page.html', 'html')
  const fetcher = vi.fn()
  vi.stubGlobal('fetch', fetcher)
  renderCard(file)

  const snapshot = () => ({
    fetchCalls: fetcher.mock.calls.length,
    inlineContent: container.querySelector(
      'iframe, object, embed, script, .markdown-message, pre',
    ),
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    fetchCalls: 0,
    inlineContent: null,
    downloads: [
      {
        text: '↓sample-page.html',
        href: file.source,
        download: 'sample-page.html',
        label: 'Download sample-page.html',
      },
    ],
  })
})

it('leaves invalid UTF-8 text as a downloadable file', async () => {
  const file = attachmentFile('sample-text.txt', 'text')
  const { fetchCalls } = await renderFetchedCard(
    file,
    new Uint8Array([0xff, 0xfe]),
    'text/plain; charset=utf-8',
  )

  const snapshot = () => ({
    fetchCalls,
    preview: container.querySelector('.syntax-code-block, .markdown-message'),
    failure: container.querySelector('.chat-file-preview-unavailable')
      ?.textContent,
    nextToDownload:
      container.querySelector('.chat-file-preview-unavailable')
        ?.nextElementSibling?.className ?? null,
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    fetchCalls: [file.source],
    preview: null,
    failure: 'Preview unavailable',
    nextToDownload: 'chat-file-download',
    downloads: [
      {
        text: '↓sample-text.txt',
        href: file.source,
        download: 'sample-text.txt',
        label: 'Download sample-text.txt',
      },
    ],
  })
})

it('shows a download-adjacent status when the attachment request fails', async () => {
  const file = attachmentFile('sample-config.json', 'text')
  const fetchCalls = await renderWithFailedFetch(file)

  const snapshot = () => ({
    fetchCalls,
    preview: container.querySelector('.syntax-code-block, .markdown-message'),
    failure: container.querySelector('.chat-file-preview-unavailable')
      ?.textContent,
    nextToDownload:
      container.querySelector('.chat-file-preview-unavailable')
        ?.nextElementSibling?.className ?? null,
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    fetchCalls: [file.source],
    preview: null,
    failure: 'Preview unavailable',
    nextToDownload: 'chat-file-download',
    downloads: [
      {
        text: '↓sample-config.json',
        href: file.source,
        download: 'sample-config.json',
        label: 'Download sample-config.json',
      },
    ],
  })
})

it('rejects HTML MIME responses before rendering Markdown and keeps the download', async () => {
  const file = attachmentFile('sample-notes.md', 'markdown')
  const { fetchCalls } = await renderFetchedCard(
    file,
    '<script>runExample()</script>',
    'text/html; charset=utf-8',
  )

  const snapshot = () => ({
    fetchCalls,
    executableContent: container.querySelector('script, iframe, object, embed'),
    markdown: container.querySelector('.markdown-message'),
    failure: container.querySelector('.chat-file-preview-unavailable')
      ?.textContent,
    nextToDownload:
      container.querySelector('.chat-file-preview-unavailable')
        ?.nextElementSibling?.className ?? null,
    downloads: downloadSnapshot(),
  })
  expect(snapshot()).toEqual({
    fetchCalls: [file.source],
    executableContent: null,
    markdown: null,
    failure: 'Preview unavailable',
    nextToDownload: 'chat-file-download',
    downloads: [
      {
        text: '↓sample-notes.md',
        href: file.source,
        download: 'sample-notes.md',
        label: 'Download sample-notes.md',
      },
    ],
  })
})
