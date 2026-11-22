import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { InboxFilterDialog } from '#components/inbox-filter-dialog'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const sessions: Session[] = Array.from({ length: 24 }, (_, index) => ({
  id: `fixture-session-${String(index)}`,
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: `Fixture question ${String(index)}`,
  preview: null,
  elapsedSeconds: index,
  unread: false,
  task: null,
  repo: 'sample-org/service',
  project: `sample-project-${String(index)}`,
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}))

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  document.documentElement.style.setProperty('--mobile-viewport-top', '0px')
  document.documentElement.style.setProperty(
    '--mobile-viewport-height',
    '320px',
  )
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  document.documentElement.style.removeProperty('--mobile-viewport-top')
  document.documentElement.style.removeProperty('--mobile-viewport-height')
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('keeps the apply button visible while long filter choices scroll', () => {
  root = createRoot(container)
  act(() => {
    root?.render(
      <InboxFilterDialog
        mobile
        sessions={sessions}
        query=""
        onQueryChange={() => undefined}
        onApply={() => undefined}
        onClose={() => undefined}
      />,
    )
  })

  const sheet = container.querySelector<HTMLElement>('.mobile-sheet')
  const options = container.querySelector<HTMLElement>('.inbox-filter-options')
  const footer = container.querySelector<HTMLElement>(
    '.inbox-filter-mobile-footer',
  )
  const applyButton = container.querySelector<HTMLButtonElement>(
    '.inbox-filter-mobile-footer button',
  )
  if (
    sheet === null ||
    options === null ||
    footer === null ||
    applyButton === null
  ) {
    throw new Error('The mobile filter controls are missing')
  }

  const snapshot = () => {
    const sheetBounds = sheet.getBoundingClientRect()
    const applyButtonBounds = applyButton.getBoundingClientRect()
    const optionsBounds = options.getBoundingClientRect()
    const footerBounds = footer.getBoundingClientRect()
    return {
      applyButtonVisible:
        applyButtonBounds.top >= sheetBounds.top &&
        applyButtonBounds.bottom <= sheetBounds.bottom,
      optionsScrollable: options.scrollHeight > options.clientHeight,
      optionsDoNotCoverFooter: optionsBounds.bottom <= footerBounds.top,
    }
  }
  expect(snapshot()).toEqual({
    applyButtonVisible: true,
    optionsScrollable: true,
    optionsDoNotCoverFooter: true,
  })
})
