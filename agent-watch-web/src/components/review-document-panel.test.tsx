import '#index.css'

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import { ReviewDocumentPanel } from '#components/review-document-panel'
import type { JsonValue } from '#types'

let container: HTMLDivElement
let root: Root | null = null
const actionSnapshots: {
  action: 'save' | 'finish'
  frontmatter: JsonValue
  body: string
}[] = []

function ReviewDocumentHarness({
  status = 'idle',
}: {
  status?: 'idle' | 'saving' | 'saved' | 'error' | 'finishing'
}) {
  const [frontmatter, setFrontmatter] = useState<JsonValue>({
    title: 'Fixture title',
    enabled: false,
    attempts: 3,
    labels: ['first', 'second'],
    retry: { limit: 2, notify: false },
  })
  const [body, setBody] = useState('## Summary\n\nFixture body.')
  const [mobileView, setMobileView] = useState<'edit' | 'preview'>('edit')

  return (
    <ReviewDocumentPanel
      frontmatter={frontmatter}
      body={body}
      status={status}
      mobileView={mobileView}
      onFrontmatterChange={setFrontmatter}
      onBodyChange={setBody}
      onMobileViewChange={setMobileView}
      onSave={() => actionSnapshots.push({ action: 'save', frontmatter, body })}
      onFinish={() =>
        actionSnapshots.push({ action: 'finish', frontmatter, body })
      }
    />
  )
}

beforeEach(() => {
  actionSnapshots.length = 0
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

it('edits typed fields and the mobile Markdown editor before saving or finishing', async () => {
  const markdownBody = Array.from(
    { length: 80 },
    (_, index) => `Fixture line ${String(index + 1)}`,
  ).join('\n')
  await page.viewport(390, 844)
  root = createRoot(container)
  act(() => root?.render(<ReviewDocumentHarness />))

  const markdownEditor = page.getByRole('textbox', { name: 'Markdown body' })
  await act(async () => {
    await page.getByRole('textbox', { name: 'title' }).fill('Edited title')
    await page.getByRole('checkbox', { name: 'enabled' }).click()
    await page.getByRole('textbox', { name: 'attempts' }).fill('4')
    await page.getByRole('textbox', { name: 'Item 1' }).fill('first edited')
    await page.getByRole('textbox', { name: 'limit' }).fill('5')
    await page.getByRole('checkbox', { name: 'notify' }).click()
    await markdownEditor.fill(markdownBody)
  })
  const editorContent = container.querySelector('.cm-content')
  const range = document.createRange()
  if (editorContent !== null) range.selectNodeContents(editorContent)
  const selection = window.getSelection()
  const scroller = container.querySelector<HTMLElement>('.cm-scroller')
  let selectedBody: string | undefined
  act(() => {
    selection?.removeAllRanges()
    if (editorContent !== null) selection?.addRange(range)
    selectedBody = selection?.toString()
    if (scroller !== null) scroller.scrollTop = 1000
  })
  const scrollState =
    scroller === null
      ? null
      : {
          overflow: getComputedStyle(scroller).overflow,
          canScroll: scroller.scrollHeight > scroller.clientHeight,
          movedToEnd: scroller.scrollTop > 0,
        }
  await act(async () => {
    await page.getByRole('button', { name: 'Preview' }).click()
  })
  const editColumn = container.querySelector('.review-document-edit-column')
  const previewColumn = container.querySelector(
    '.review-document-preview-column',
  )
  const input = container.querySelector<HTMLInputElement>(
    '.review-document-input',
  )
  await act(async () => {
    await page.getByRole('button', { name: 'Save' }).click()
    await page.getByRole('button', { name: 'Finish review' }).click()
  })

  const snapshot = () => ({
    snapshots: actionSnapshots,
    mobileView: container
      .querySelector('.review-document-panel')
      ?.getAttribute('data-mobile-view'),
    contentEditable: container
      .querySelector('.review-document-editor .cm-content')
      ?.getAttribute('contenteditable'),
    mobileColumns: [
      editColumn === null ? null : getComputedStyle(editColumn).display,
      previewColumn === null ? null : getComputedStyle(previewColumn).display,
    ],
    mobileInputSize: input === null ? null : getComputedStyle(input).fontSize,
    selectedBody,
    scrollState,
  })

  expect(snapshot()).toEqual({
    snapshots: [
      {
        action: 'save',
        frontmatter: {
          title: 'Edited title',
          enabled: true,
          attempts: 4,
          labels: ['first edited', 'second'],
          retry: { limit: 5, notify: true },
        },
        body: markdownBody,
      },
      {
        action: 'finish',
        frontmatter: {
          title: 'Edited title',
          enabled: true,
          attempts: 4,
          labels: ['first edited', 'second'],
          retry: { limit: 5, notify: true },
        },
        body: markdownBody,
      },
    ],
    mobileView: 'preview',
    contentEditable: 'true',
    mobileColumns: ['none', 'block'],
    mobileInputSize: '16px',
    selectedBody: markdownBody,
    scrollState: {
      overflow: 'auto',
      canScroll: true,
      movedToEnd: true,
    },
  })
})

it.each([
  { device: 'mobile', width: 390 },
  { device: 'desktop', width: 1280 },
])(
  'keeps the review actions visible while document content scrolls at $device width',
  async ({ width }) => {
    await page.viewport(width, 844)
    container.style.width = `${String(width)}px`
    container.style.height = '360px'
    root = createRoot(container)
    act(() =>
      root?.render(
        <ReviewDocumentPanel
          frontmatter={Object.fromEntries(
            Array.from({ length: 20 }, (_, index) => [
              `field-${String(index + 1)}`,
              `value-${String(index + 1)}`,
            ]),
          )}
          body={Array.from(
            { length: 80 },
            (_, index) => `Document line ${String(index + 1)}`,
          ).join('\n')}
        />,
      ),
    )

    const panel = container.querySelector<HTMLElement>('.review-document-panel')
    const content = container.querySelector<HTMLElement>(
      '.review-document-content',
    )
    const footer = container.querySelector<HTMLElement>(
      '.review-document-actions',
    )
    if (panel === null || content === null || footer === null) {
      throw new Error('Review document layout elements were not rendered.')
    }

    const readActionBounds = () => {
      const panelRect = panel.getBoundingClientRect()
      const footerRect = footer.getBoundingClientRect()
      const actionButtonRects = Array.from(
        footer.querySelectorAll<HTMLButtonElement>('button'),
        (button) => button.getBoundingClientRect(),
      )

      return {
        footer: { top: footerRect.top, bottom: footerRect.bottom },
        footerWithinPanel:
          footerRect.top >= panelRect.top &&
          footerRect.bottom <= panelRect.bottom,
        buttons: actionButtonRects.map((rect) => ({
          top: rect.top,
          bottom: rect.bottom,
          withinPanel:
            rect.top >= panelRect.top && rect.bottom <= panelRect.bottom,
        })),
      }
    }

    const boundsAtStart = readActionBounds()
    act(() => {
      content.scrollTop = content.scrollHeight
    })
    const boundsAtEnd = readActionBounds()
    const maxScrollTop = content.scrollHeight - content.clientHeight

    const snapshot = () => ({
      contentOverflowY: getComputedStyle(content).overflowY,
      contentCanScroll: maxScrollTop > 0,
      contentReachedEnd: content.scrollTop >= maxScrollTop,
      footerWithinPanelAtStart: boundsAtStart.footerWithinPanel,
      footerWithinPanelAtEnd: boundsAtEnd.footerWithinPanel,
      actionButtonsWithinPanelAtStart: boundsAtStart.buttons.map(
        (button) => button.withinPanel,
      ),
      actionButtonsWithinPanelAtEnd: boundsAtEnd.buttons.map(
        (button) => button.withinPanel,
      ),
      footerStayedInPlace:
        boundsAtStart.footer.top === boundsAtEnd.footer.top &&
        boundsAtStart.footer.bottom === boundsAtEnd.footer.bottom,
      actionButtonsStayedInPlace: boundsAtStart.buttons.every(
        (button, index) =>
          button.top === boundsAtEnd.buttons[index].top &&
          button.bottom === boundsAtEnd.buttons[index].bottom,
      ),
    })

    expect(snapshot()).toEqual({
      contentOverflowY: 'auto',
      contentCanScroll: true,
      contentReachedEnd: true,
      footerWithinPanelAtStart: true,
      footerWithinPanelAtEnd: true,
      actionButtonsWithinPanelAtStart: [true, true],
      actionButtonsWithinPanelAtEnd: [true, true],
      footerStayedInPlace: true,
      actionButtonsStayedInPlace: true,
    })
  },
)

it('disables frontmatter and Markdown editing while a request is in progress', async () => {
  root = createRoot(container)
  act(() => root?.render(<ReviewDocumentHarness status="saving" />))
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const stateSnapshot = () => ({
    frontmatterInputs: Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.review-document-frontmatter input',
      ),
      (input) => input.disabled,
    ),
    markdownEditable: container
      .querySelector('.review-document-editor .cm-content')
      ?.getAttribute('contenteditable'),
    saveDisabled: container.querySelector<HTMLButtonElement>(
      '.review-document-actions button',
    )?.disabled,
    finishDisabled: container.querySelector<HTMLButtonElement>(
      '.review-document-finish',
    )?.disabled,
  })

  expect(stateSnapshot()).toEqual({
    frontmatterInputs: [true, true, true, true, true, true, true],
    markdownEditable: 'false',
    saveDisabled: true,
    finishDisabled: true,
  })
})
