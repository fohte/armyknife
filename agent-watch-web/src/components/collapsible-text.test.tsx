import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { CollapsibleText } from '#components/collapsible-text'

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
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('offers to expand text after its available width shrinks', async () => {
  const frame = document.createElement('div')
  frame.style.width = '800px'
  container.append(frame)
  root = createRoot(frame)
  act(() => {
    root?.render(
      <CollapsibleText expanded={false} onExpandedChange={() => undefined}>
        This note fits in one line at a wide size but wraps to several lines
        when the card narrows. Keep this extra sentence to ensure the content
        needs more than two lines at a narrow width.
      </CollapsibleText>,
    )
  })

  const content = container.querySelector<HTMLDivElement>(
    '.collapsible-text__content',
  )
  if (content === null) throw new Error('The text content was not rendered')

  const initialToggle = container.querySelector(
    '.collapsible-text__toggle',
  )?.textContent
  await act(async () => {
    frame.style.width = '180px'
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          resolve()
        })
      })
    })
  })
  const toggle = container.querySelector<HTMLButtonElement>(
    '.collapsible-text__toggle',
  )

  const snapshot = () => ({
    initialToggle,
    resizedToggle: toggle?.textContent,
    expanded: toggle?.getAttribute('aria-expanded'),
  })

  expect(snapshot()).toEqual({
    initialToggle: undefined,
    resizedToggle: 'Show more',
    expanded: 'false',
  })
})
