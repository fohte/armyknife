import '#index.css'

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import { ReviewDocumentPanel } from '#components/review-document-panel'
import { SessionStatusPanel } from '#components/session-status-panel'
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
  const [view, setView] = useState<'edit' | 'preview'>('preview')

  return (
    <div
      className="session-status-panel"
      data-size="phone"
      style={{ width: '390px', height: '844px', display: 'flex' }}
    >
      <ReviewDocumentPanel
        frontmatter={frontmatter}
        body={body}
        status={status}
        view={view}
        onFrontmatterChange={setFrontmatter}
        onBodyChange={setBody}
        onViewChange={setView}
        onSave={() =>
          actionSnapshots.push({ action: 'save', frontmatter, body })
        }
        onFinish={() =>
          actionSnapshots.push({ action: 'finish', frontmatter, body })
        }
      />
    </div>
  )
}

function ExpandedReviewHarness() {
  const [expanded, setExpanded] = useState(true)
  const summary = {
    id: 'fixture-pr-draft',
    kind: 'pr_draft' as const,
    target: { repository: 'example/service', branch: 'retry-policy' },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }

  return (
    <SessionStatusPanel
      tabs={[
        {
          id: 'review-document',
          label: 'Review',
          count: 1,
          visible: true,
          showCount: true,
          content: (
            <ReviewDocumentPanel
              summary={summary}
              summaries={[summary]}
              isExpanded={expanded}
              frontmatter={{
                title: 'Fixture title',
                labels: ['reliability'],
              }}
              body="Fixture body."
              view="edit"
              onExpand={() => {
                setExpanded((current) => !current)
              }}
            />
          ),
        },
      ]}
      activeTabId="review-document"
      expanded={expanded}
      showCollapseControl={false}
      onTabChange={() => undefined}
      onClose={() => undefined}
      onCollapse={() => {
        setExpanded(false)
      }}
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
    await page.getByRole('button', { name: 'Edit' }).click()
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
    view: container
      .querySelector('.review-document-panel')
      ?.getAttribute('data-view'),
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
    view: 'preview',
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

it('shows the unsaved state before the review actions', () => {
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        frontmatter={{ title: 'Fixture title' }}
        body="Fixture body."
        unsavedChanges
      />,
    ),
  )

  expect(container.querySelector('.review-document-status')?.textContent).toBe(
    'Unsaved changes',
  )
})

it('shows the only open draft in the draft tab row', () => {
  const summary = {
    id: 'fixture-issue-comment',
    kind: 'issue_comment' as const,
    target: { issueNumber: 28, title: 'Improve retries' },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={summary}
        summaries={[summary]}
        frontmatter={{}}
        body="Fixture body."
      />,
    ),
  )

  const tab = container.querySelector('.review-document-tab')
  const snapshot = () => ({
    label: tab?.getAttribute('aria-label'),
    current: tab?.getAttribute('aria-current'),
    status: tab
      ?.querySelector('.review-document-dot')
      ?.getAttribute('aria-label'),
  })

  expect(snapshot()).toEqual({
    label: 'Issue comment #28 · Improve retries',
    current: 'page',
    status: 'Editor open',
  })
})

it('omits the frontmatter section when a PR draft has only its title', () => {
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={{
          id: 'fixture-pr-draft',
          kind: 'pr_draft',
          target: { repository: 'example/service', branch: 'retry-policy' },
          isEditor: true,
          canSave: true,
          canFinish: true,
        }}
        frontmatter={{ title: 'Fixture title' }}
        body="Fixture body."
      />,
    ),
  )

  const snapshot = () => ({
    frontmatterHeading: container.querySelector(
      '.review-document-frontmatter h2',
    )?.textContent,
    title: container.querySelector<HTMLInputElement>(
      'input[aria-label="Draft title"]',
    )?.value,
  })
  expect(snapshot()).toEqual({
    frontmatterHeading: undefined,
    title: 'Fixture title',
  })
})

it('shows both editable columns and one full width control when expanded', async () => {
  await page.viewport(800, 900)
  container.style.width = '800px'
  container.style.height = '900px'
  container.style.display = 'flex'
  root = createRoot(container)
  act(() => root?.render(<ExpandedReviewHarness />))

  const panel = container.querySelector<HTMLElement>('.review-document-panel')
  const columns = container.querySelector<HTMLElement>(
    '.review-document-body-columns',
  )
  const editColumn = container.querySelector<HTMLElement>(
    '.review-document-edit-column',
  )
  const previewColumn = container.querySelector<HTMLElement>(
    '.review-document-preview-column',
  )
  const frontmatter = container.querySelector<HTMLElement>(
    '.review-document-frontmatter',
  )
  if (
    panel === null ||
    columns === null ||
    editColumn === null ||
    previewColumn === null ||
    frontmatter === null
  ) {
    throw new Error('The expanded review layout was not rendered.')
  }

  const snapshot = () => ({
    expanded: panel.getAttribute('data-expanded'),
    columns: getComputedStyle(columns).display,
    editColumn: getComputedStyle(editColumn).display,
    previewColumn: getComputedStyle(previewColumn).display,
    frontmatter: getComputedStyle(frontmatter).display,
    headings: Array.from(
      container.querySelectorAll('.review-document-column-heading'),
      (heading) => heading.textContent,
    ),
    fullWidthControls: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.review-document-expand, .session-status-panel-collapse',
      ),
      (button) => button.getAttribute('aria-label'),
    ),
    metadataValue: container.querySelector<HTMLInputElement>(
      '.review-document-frontmatter input[aria-label="Item 1"]',
    )?.value,
  })

  expect(snapshot()).toEqual({
    expanded: 'true',
    columns: 'grid',
    editColumn: 'flex',
    previewColumn: 'block',
    frontmatter: 'flex',
    headings: ['Markdown', 'Preview'],
    fullWidthControls: ['Exit full width review'],
    metadataValue: 'reliability',
  })
})

it('collapses the expanded Review panel from its single header control', async () => {
  await page.viewport(800, 900)
  container.style.width = '800px'
  container.style.height = '900px'
  container.style.display = 'flex'
  root = createRoot(container)
  act(() => root?.render(<ExpandedReviewHarness />))

  await act(async () => {
    await page.getByRole('button', { name: 'Exit full width review' }).click()
  })

  const panel = container.querySelector('.review-document-panel')
  const control = container.querySelector<HTMLButtonElement>(
    '.review-document-expand',
  )
  const snapshot = () => ({
    expanded: panel?.getAttribute('data-expanded'),
    label: control?.getAttribute('aria-label'),
    pressed: control?.getAttribute('aria-pressed'),
    controlCount: container.querySelectorAll(
      '.review-document-expand, .session-status-panel-collapse',
    ).length,
  })
  expect(snapshot()).toEqual({
    expanded: 'false',
    label: 'Open draft full width',
    pressed: 'false',
    controlCount: 1,
  })
})

it('keeps a queued draft readable while disabling editing and review actions', async () => {
  await page.viewport(390, 844)
  const summary = {
    id: 'fixture-queued-draft',
    kind: 'draft' as const,
    target: {},
    isEditor: false,
    canSave: false,
    canFinish: false,
  }
  const onSave = vi.fn()
  const onFinish = vi.fn()
  function QueuedDraftHarness() {
    const [view, setView] = useState<'edit' | 'preview'>('preview')
    return (
      <ReviewDocumentPanel
        summary={summary}
        summaries={[summary]}
        canSave={false}
        canFinish={false}
        frontmatter={{ title: 'Queued title', enabled: true }}
        body="Queued draft body."
        view={view}
        onViewChange={setView}
        onSave={onSave}
        onFinish={onFinish}
      />
    )
  }

  root = createRoot(container)
  act(() => root?.render(<QueuedDraftHarness />))
  const preview = {
    metadataInputsDisabled: Array.from(
      container.querySelectorAll<HTMLInputElement>(
        '.review-document-frontmatter input',
      ),
      (input) => input.disabled,
    ),
    saveDisabled: container.querySelector<HTMLButtonElement>(
      '.review-document-actions button:not(.review-document-finish)',
    )?.disabled,
    finishDisabled: container.querySelector<HTMLButtonElement>(
      '.review-document-finish',
    )?.disabled,
    body: container.querySelector('.review-document-body')?.textContent,
  }

  await act(async () => {
    await page.getByRole('button', { name: 'Edit' }).click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>(
        '.review-document-actions button:not(.review-document-finish)',
      )
      ?.click()
    container
      .querySelector<HTMLButtonElement>('.review-document-finish')
      ?.click()
  })

  const frontmatter = container.querySelector<HTMLElement>(
    '.review-document-frontmatter',
  )
  const snapshot = () => ({
    preview,
    edit: {
      view: container
        .querySelector('.review-document-panel')
        ?.getAttribute('data-view'),
      frontmatterDisplay:
        frontmatter === null ? null : getComputedStyle(frontmatter).display,
      metadataInputsDisabled: Array.from(
        container.querySelectorAll<HTMLInputElement>(
          '.review-document-frontmatter input',
        ),
        (input) => input.disabled,
      ),
      bodyEditorDisabled:
        container
          .querySelector('.review-document-editor .cm-content')
          ?.getAttribute('contenteditable') === 'false',
      saveDisabled: container.querySelector<HTMLButtonElement>(
        '.review-document-actions button:not(.review-document-finish)',
      )?.disabled,
      finishDisabled: container.querySelector<HTMLButtonElement>(
        '.review-document-finish',
      )?.disabled,
    },
    callbacks: {
      save: onSave.mock.calls.length,
      finish: onFinish.mock.calls.length,
    },
  })
  expect(snapshot()).toEqual({
    preview: {
      metadataInputsDisabled: [true, true],
      saveDisabled: true,
      finishDisabled: true,
      body: 'Queued draft body.',
    },
    edit: {
      view: 'edit',
      frontmatterDisplay: 'flex',
      metadataInputsDisabled: [true, true],
      bodyEditorDisabled: true,
      saveDisabled: true,
      finishDisabled: true,
    },
    callbacks: { save: 0, finish: 0 },
  })
})

it('keeps the mobile preview usable when metadata fills the viewport', async () => {
  await page.viewport(390, 844)
  container.style.width = '390px'
  container.style.height = '844px'
  container.style.display = 'flex'
  root = createRoot(container)
  act(() =>
    root?.render(
      <SessionStatusPanel
        tabs={[
          {
            id: 'review-document',
            label: 'Review',
            count: 0,
            visible: true,
            showCount: false,
            content: (
              <ReviewDocumentPanel
                frontmatter={Object.fromEntries(
                  Array.from({ length: 20 }, (_, index) => [
                    `field-${String(index + 1)}`,
                    `value-${String(index + 1)}`,
                  ]),
                )}
                body="A review document with a long body."
              />
            ),
          },
        ]}
        activeTabId="review-document"
        onTabChange={() => undefined}
        onClose={() => undefined}
        size="phone"
      />,
    ),
  )

  const bodyEditor = container.querySelector<HTMLElement>(
    '.review-document-body-editor',
  )
  const metadata = container.querySelector<HTMLElement>(
    '.review-document-frontmatter',
  )
  const previewColumn = container.querySelector<HTMLElement>(
    '.review-document-preview-column',
  )
  if (bodyEditor === null || metadata === null || previewColumn === null) {
    throw new Error('The mobile review editor layout was not rendered.')
  }

  const snapshot = () => ({
    bodyEditorHasRoom: bodyEditor.getBoundingClientRect().height >= 320,
    previewColumnHasRoom: previewColumn.getBoundingClientRect().height >= 240,
    metadataCanScroll: metadata.scrollHeight > metadata.clientHeight,
  })
  expect(snapshot()).toEqual({
    bodyEditorHasRoom: true,
    previewColumnHasRoom: true,
    metadataCanScroll: true,
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
    container.style.display = 'flex'
    root = createRoot(container)
    act(() =>
      root?.render(
        <div
          className="session-status-panel"
          data-size={width === 390 ? 'phone' : 'desktop'}
          style={{ width: '100%', height: '100%', display: 'flex' }}
        >
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
            view="edit"
          />
        </div>,
      ),
    )

    const panel = container.querySelector<HTMLElement>('.review-document-panel')
    const scroller = container.querySelector<HTMLElement>('.cm-scroller')
    const footer = container.querySelector<HTMLElement>(
      '.review-document-actions',
    )
    if (panel === null || scroller === null || footer === null) {
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
      scroller.scrollTop = scroller.scrollHeight
    })
    const boundsAtEnd = readActionBounds()
    const maxScrollTop = scroller.scrollHeight - scroller.clientHeight

    const snapshot = () => ({
      editorOverflowY: getComputedStyle(scroller).overflowY,
      contentCanScroll: maxScrollTop > 0,
      contentReachedEnd: scroller.scrollTop >= maxScrollTop,
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
      actionButtonsStayedInPlace:
        boundsAtStart.buttons.length === boundsAtEnd.buttons.length &&
        boundsAtStart.buttons.every((button, index) => {
          const endButton = boundsAtEnd.buttons[index]
          return (
            endButton !== undefined &&
            button.top === endButton.top &&
            button.bottom === endButton.bottom
          )
        }),
    })

    expect(snapshot()).toEqual({
      editorOverflowY: 'auto',
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
