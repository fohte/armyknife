import '#components/file-attachment-card.css'

import { useState } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import { SyntaxCodeBlock } from '#components/syntax-code-block'
import type { ChatItem } from '#types'

type FileAttachmentMessage = Extract<ChatItem, { type: 'file_attachment' }>
export type AttachmentPreviewFile = FileAttachmentMessage['files'][number]

const syntaxLanguages: Record<string, string> = {
  bash: 'bash',
  c: 'c',
  cc: 'cpp',
  cpp: 'cpp',
  cs: 'csharp',
  css: 'css',
  go: 'go',
  html: 'html',
  java: 'java',
  js: 'javascript',
  json: 'json',
  jsx: 'jsx',
  kt: 'kotlin',
  lua: 'lua',
  md: 'markdown',
  mjs: 'javascript',
  py: 'python',
  rb: 'ruby',
  rs: 'rust',
  sh: 'bash',
  sql: 'sql',
  toml: 'toml',
  ts: 'typescript',
  tsx: 'tsx',
  xml: 'xml',
  yaml: 'yaml',
  yml: 'yaml',
}

function syntaxLanguage(name: string): string {
  const extension = name.split('.').at(-1)?.toLowerCase()
  return extension === undefined
    ? 'text'
    : (syntaxLanguages[extension] ?? 'text')
}

function TextPreview({
  file,
  content,
  loading,
  deferred,
  onPreviewOpen,
}: {
  file: AttachmentPreviewFile
  content: string | undefined
  loading: boolean
  deferred: boolean
  onPreviewOpen: () => void
}) {
  const preview =
    content === undefined ? (
      loading ? (
        <span className="chat-file-preview-loading">Loading preview…</span>
      ) : null
    ) : file.previewType === 'markdown' ? (
      <MarkdownMessage className="chat-file-markdown-preview">
        {content}
      </MarkdownMessage>
    ) : (
      <SyntaxCodeBlock
        className="chat-file-text-preview"
        code={content}
        language={syntaxLanguage(file.name)}
      />
    )

  return deferred ? (
    <details
      className="chat-file-collapsed-preview"
      onToggle={(event) => {
        if (event.currentTarget.open) onPreviewOpen()
      }}
    >
      <summary>Show file contents</summary>
      {preview}
    </details>
  ) : (
    preview
  )
}

export function FileContentPreview({
  file,
  textContent,
  loading = false,
  deferred = false,
  onPreviewOpen = () => undefined,
  previewUnavailable = false,
  onEnlargeImage = () => undefined,
}: {
  file: AttachmentPreviewFile
  textContent?: string
  loading?: boolean
  deferred?: boolean
  onPreviewOpen?: () => void
  previewUnavailable?: boolean
  onEnlargeImage?: (source: string) => void
}) {
  const [imageFailed, setImageFailed] = useState(false)

  if (!file.available) {
    return (
      <span className="chat-file-unavailable">{file.name} · unavailable</span>
    )
  }

  const preview = (() => {
    switch (file.previewType) {
      case 'image':
        return imageFailed ? null : (
          <button
            className="chat-file-image-button"
            type="button"
            aria-label={`Enlarge ${file.name}`}
            onClick={() => {
              onEnlargeImage(file.source)
            }}
          >
            <img
              src={file.source}
              alt={file.name}
              loading="lazy"
              onError={() => {
                setImageFailed(true)
              }}
            />
          </button>
        )
      case 'markdown':
      case 'text':
        return (
          <TextPreview
            file={file}
            content={textContent}
            loading={loading}
            deferred={deferred}
            onPreviewOpen={onPreviewOpen}
          />
        )
      case 'video':
        return (
          <video
            className="chat-file-media-preview"
            src={file.source}
            controls
            preload="metadata"
          />
        )
      case 'audio':
        return (
          <audio
            className="chat-file-media-preview"
            src={file.source}
            controls
            preload="metadata"
          />
        )
      case 'pdf':
        return (
          <a
            className="chat-file-open-pdf"
            href={file.source}
            target="_blank"
            rel="noopener noreferrer"
          >
            Open PDF
          </a>
        )
      case 'html':
      case 'download':
        return null
    }
  })()

  return (
    <>
      {preview}
      {previewUnavailable && (
        <span className="chat-file-preview-unavailable" role="status">
          Preview unavailable
        </span>
      )}
      <a
        className="chat-file-download"
        href={file.source}
        download={file.name}
        aria-label={`Download ${file.name}`}
      >
        <span aria-hidden="true">↓</span>
        {file.name}
      </a>
    </>
  )
}
