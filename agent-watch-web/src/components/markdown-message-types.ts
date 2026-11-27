export type MarkdownNode = {
  children?: MarkdownNode[]
  position?: {
    start: { offset?: number }
    end: { offset?: number }
  }
  tagName?: string
  type: string
  url?: string
  value?: string
  data?: {
    hName?: string
    hProperties?: Record<string, boolean | string>
  }
}

export type MarkdownRoot = MarkdownNode & {
  children: MarkdownNode[]
}

export type MarkdownFile = {
  value: string | Uint8Array
}
