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

function ReviewDocumentHarness() {
  const [frontmatter, setFrontmatter] = useState<JsonValue>({
    title: 'Fixture title',
    enabled: false,
    attempts: 3,
    labels: ['first', 'second'],
  })
  const [body, setBody] = useState('## Summary\n\nFixture body.')
  const [mobileView, setMobileView] = useState<'edit' | 'preview'>('edit')

  return (
    <ReviewDocumentPanel
      frontmatter={frontmatter}
      body={body}
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
