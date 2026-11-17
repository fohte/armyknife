import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { ToolSummary } from '#components/tool-summary'

let container: HTMLDivElement
let root: Root | null = null

function getThoughtRowState(
  container: HTMLDivElement,
  thoughtDetails: HTMLDetailsElement,
) {
  const outerSummary = container.querySelector('.chat-tool-summary-text')
  const label = thoughtDetails.querySelector('.chat-tool-name')
  const length = thoughtDetails.querySelector('.chat-tool-thought-length')
  const preview = thoughtDetails.querySelector('.chat-tool-thought-preview')
  const fullText = thoughtDetails.querySelector('.chat-tool-thought-full')

  return {
    width: container.getBoundingClientRect().width,
    groupHeading: outerSummary?.textContent ?? null,
    thoughtOpen: thoughtDetails.open,
    label: label?.textContent ?? null,
    labelFits: label !== null && label.scrollWidth <= label.clientWidth,
    characterCount: length?.textContent ?? null,
    characterCountDisplay:
      length === null ? null : getComputedStyle(length).display,
    previewDisplay: preview === null ? null : getComputedStyle(preview).display,
    fullText: fullText?.textContent ?? null,
    fullTextDisplay:
      fullText === null ? null : getComputedStyle(fullText).display,
  }
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.style.width = '320px'
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('shows an expanded thought once and keeps its label visible at mobile width', async () => {
  const thoughtText = 'Review 👩‍💻.'
  root = createRoot(container)
  act(() => {
    root?.render(
      <ToolSummary
        summary="Thought ×1 · Read ×1"
        operations={[]}
        items={[
          { type: 'thinking', text: thoughtText },
          {
            type: 'tool_use',
            tool: 'Read',
            target: 'fixture/source.rs',
            failed: false,
          },
        ]}
        open
      />,
    )
  })

  const thoughtDetails = container.querySelector<HTMLDetailsElement>(
    '.chat-tool-thought details',
  )
  const thoughtSummary = thoughtDetails?.querySelector('summary') ?? null
  if (thoughtDetails === null || thoughtSummary === null) {
    throw new Error('Expected the grouped thought row')
  }

  await act(async () => userEvent.click(thoughtSummary))

  expect(getThoughtRowState(container, thoughtDetails)).toEqual({
    width: 320,
    groupHeading: '1 thought · 1 tool call',
    thoughtOpen: true,
    label: 'Thought',
    labelFits: true,
    characterCount: '9 chars',
    characterCountDisplay: 'block',
    previewDisplay: 'none',
    fullText: thoughtText,
    fullTextDisplay: 'block',
  })
})
