import type { Meta, StoryObj } from '@storybook/react-vite'

import { FileContentPreview } from '#components/file-content-preview'

const markdownFile = {
  name: 'sample-notes.md',
  source: '/api/sessions/fixture-session/files/fixture-markdown',
  size: 48,
  isImage: false,
  previewType: 'markdown',
  available: true,
} as const

const textFile = {
  name: 'sample-config.json',
  source: '/api/sessions/fixture-session/files/fixture-text',
  size: 32,
  isImage: false,
  previewType: 'text',
  available: true,
} as const

const meta = {
  component: FileContentPreview,
  args: {
    file: markdownFile,
    textContent: '# Sample notes\n\nThe **example** is ready.',
  },
  decorators: [
    (Story) => (
      <div
        style={{ maxWidth: '520px', padding: '16px', background: '#0a0a0a' }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FileContentPreview>

export default meta
type Story = StoryObj<typeof meta>

export const MarkdownUsesConversationFormatting: Story = {
  name: 'Markdown uses the same headings and emphasis as a message',
}

export const ImageKeepsItsDownloadLink: Story = {
  name: 'An inline image keeps a separate download link',
  args: {
    file: {
      name: 'sample-image.png',
      source: 'data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=',
      size: 68,
      isImage: true,
      previewType: 'image',
      available: true,
    },
  },
}

export const TextUsesASyntaxCodeBlock: Story = {
  name: 'JSON text stays literal inside a syntax code block',
  args: {
    file: textFile,
    textContent: '{"mode":"sample","ready":true}',
  },
}

export const LongTextStartsCollapsed: Story = {
  name: 'Long text starts collapsed and can be expanded',
  args: {
    deferred: true,
    file: { ...textFile, name: 'sample-output.log', size: 3600 },
    textContent: Array.from(
      { length: 180 },
      (_, index) => `Example output line ${String(index + 1)}.`,
    ).join('\n'),
  },
}

export const VideoHasPlaybackAndSeekControls: Story = {
  name: 'A video attachment has playback and seeking controls',
  args: {
    file: {
      name: 'sample-clip.mp4',
      source: 'data:video/mp4;base64,AA==',
      size: 2048,
      isImage: false,
      previewType: 'video',
      available: true,
    },
  },
}

export const AudioHasPlaybackControls: Story = {
  name: 'An audio attachment has playback controls',
  args: {
    file: {
      name: 'sample-audio.mp3',
      source: 'data:audio/mpeg;base64,AA==',
      size: 2048,
      isImage: false,
      previewType: 'audio',
      available: true,
    },
  },
}

export const PdfOpensInTheBrowser: Story = {
  name: 'A PDF attachment offers an inline browser viewer',
  args: {
    file: {
      name: 'sample-report.pdf',
      source: '/api/sessions/fixture-session/files/fixture-pdf',
      size: 2048,
      isImage: false,
      previewType: 'pdf',
      available: true,
    },
  },
}

export const HtmlHasNoInlinePreview: Story = {
  name: 'HTML content has no inline preview',
  args: {
    file: {
      name: 'sample-page.html',
      source: '/api/sessions/fixture-session/files/fixture-html',
      size: 384,
      isImage: false,
      previewType: 'html',
      available: true,
    },
  },
}

export const UnrecognizedFileHasNoInlinePreview: Story = {
  name: 'An unrecognized file remains available as a download',
  args: {
    file: {
      name: 'sample-shortcut.shortcut',
      source: '/api/sessions/fixture-session/files/fixture-shortcut',
      size: 32,
      isImage: false,
      previewType: 'download',
      available: true,
    },
  },
}

export const PreviewFailureKeepsTheDownloadAvailable: Story = {
  name: 'A failed text preview shows its status beside the download',
  args: {
    file: textFile,
    textContent: '',
    previewUnavailable: true,
  },
}
