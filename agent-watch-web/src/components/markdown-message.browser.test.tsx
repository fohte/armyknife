import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { MarkdownMessage } from '#components/markdown-message'

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

function getMarkdownLayoutSnapshot() {
  const message = container.firstElementChild
  const paragraph = container.querySelector('p')
  const range = document.createRange()
  if (message === null || paragraph === null) {
    throw new Error('Expected Markdown message blocks to render')
  }
  range.selectNodeContents(paragraph)
  const blocks = Array.from(message.children)
  const blockGaps = blocks.slice(1).map((block, index) => {
    const previous = blocks[index]
    if (previous === undefined) throw new Error('Expected a preceding block')
    return Math.round(
      block.getBoundingClientRect().top -
        previous.getBoundingClientRect().bottom,
    )
  })
  const paragraphLineCount = new Set(
    Array.from(range.getClientRects(), (rect) => Math.round(rect.top)),
  ).size

  return [
    getComputedStyle(message).whiteSpace,
    getComputedStyle(paragraph).whiteSpace,
    paragraphLineCount,
    blocks.map((block) => block.tagName),
    blockGaps,
  ]
}

it('preserves paragraph line breaks without adding blank space between blocks', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <MarkdownMessage>{`## Retry policy

First line
Second line

- Retry transient failures
- Stop after three attempts

| Attempt | Delay |
| --- | --- |
| 1 | 100 ms |

~~~rust
let attempts = 3;
~~~`}</MarkdownMessage>,
    )
  })

  expect(getMarkdownLayoutSnapshot()).toEqual([
    'normal',
    'pre-line',
    2,
    ['H2', 'P', 'UL', 'TABLE', 'PRE'],
    [12, 12, 12, 12],
  ])
})
