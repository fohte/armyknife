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

it('shows the issue heading without a draft tab row when one draft is open', () => {
  const summary = {
    id: 'fixture-issue-comment',
    kind: 'issue_comment' as const,
    target: {
      repository: 'example/web-app',
      issueNumber: 412,
      title: 'Delivery checks fail on the fourth attempt',
    },
    startedAt: Date.now() - 180_000,
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

  const snapshot = () => ({
    tabs: container.querySelectorAll('.review-document-tabs').length,
    kind: container.querySelector('.review-document-kind')?.textContent,
    target: container.querySelector('.review-document-target')?.textContent,
    title: container.querySelector('.review-document-title')?.textContent,
    description: container.querySelector('.review-document-description')
      ?.textContent,
    status: container.querySelector('.review-document-editor-status')
      ?.textContent,
  })

  expect(snapshot()).toEqual({
    tabs: 0,
    kind: 'Issue comment',
    target: 'example/web-app #412',
    title: '#412 Delivery checks fail on the fourth attempt',
    description: 'New comment on this issue',
    status: 'In the editor · 3m',
  })
})

it('shows flat tabs with square editor and queued markers for multiple drafts', () => {
  const summaries = [
    {
      id: 'fixture-issue-comment',
      kind: 'issue_comment' as const,
      target: { repository: 'example/web-app', issueNumber: 412 },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    {
      id: 'fixture-pr-draft',
      kind: 'pr_draft' as const,
      target: { repository: 'example/web-app', branch: 'feature/retry-limit' },
      isEditor: false,
      canSave: false,
      canFinish: false,
    },
    {
      id: 'fixture-review-replies',
      kind: 'review_replies' as const,
      target: { pullRequestNumber: 2049, threadCount: 2 },
      isEditor: false,
      canSave: false,
      canFinish: false,
    },
    {
      id: 'fixture-draft',
      kind: 'draft' as const,
      target: { path: 'docs/retry-plan.md' },
      isEditor: false,
      canSave: false,
      canFinish: false,
    },
  ]
  const firstSummary = summaries[0]
  if (firstSummary === undefined) {
    throw new Error('The issue comment fixture was not created.')
  }
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={firstSummary}
        summaries={summaries}
        frontmatter={{}}
        body="Fixture body."
      />,
    ),
  )

  const snapshot = () => ({
    order: Array.from(
      container.querySelector('.review-document-panel')?.children ?? [],
      (child) => child.className,
    ),
    role: container
      .querySelector('.review-document-tabs')
      ?.getAttribute('role'),
    tabs: Array.from(
      container.querySelectorAll<HTMLButtonElement>('.review-document-tab'),
      (tab) => ({
        selected: tab.getAttribute('aria-selected'),
        kind: tab.querySelector('.review-document-tab-kind')?.textContent,
        target: tab.querySelector('.review-document-tab-target')?.textContent,
        marker: tab.querySelector('.review-document-dot')?.className,
      }),
    ),
  })

  expect(snapshot()).toEqual({
    order: [
      'review-document-tabs',
      'review-document-header',
      'review-document-content',
      'review-document-actions',
    ],
    role: 'tablist',
    tabs: [
      {
        selected: 'true',
        kind: 'Issue comment',
        target: '#412',
        marker: 'review-document-dot review-document-dot--editor',
      },
      {
        selected: 'false',
        kind: 'PR draft',
        target: 'feature/retry-limit',
        marker: 'review-document-dot',
      },
      {
        selected: 'false',
        kind: 'Review replies',
        target: 'PR #2049',
        marker: 'review-document-dot',
      },
      {
        selected: 'false',
        kind: 'Draft',
        target: 'retry-plan.md',
        marker: 'review-document-dot',
      },
    ],
  })
})

it('places the Full width action before Preview and Edit without framing the preview', () => {
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={{
          id: 'fixture-issue-comment',
          kind: 'issue_comment',
          target: { repository: 'example/web-app', issueNumber: 412 },
          isEditor: true,
          canSave: true,
          canFinish: true,
        }}
        summaries={[]}
        frontmatter={{}}
        body="Fixture body."
      />,
    ),
  )

  const preview = container.querySelector<HTMLElement>(
    '.review-document-preview-column',
  )
  const fullWidthButton = container.querySelector<HTMLButtonElement>(
    '.review-document-full-width',
  )
  if (preview === null || fullWidthButton === null) {
    throw new Error('The draft body toolbar was not rendered.')
  }
  const snapshot = () => ({
    buttons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.review-document-body-toolbar button',
      ),
      (button) => ({
        label: button.getAttribute('aria-label'),
        text: button.textContent,
      }),
    ),
    previewPadding: getComputedStyle(preview).padding,
    previewBorderWidth: getComputedStyle(preview).borderWidth,
    previewSelected: container
      .querySelector('.review-document-view-toggle button')
      ?.getAttribute('aria-pressed'),
  })
  expect(snapshot()).toEqual({
    buttons: [
      { label: 'Open draft full width', text: 'Full width' },
      { label: null, text: 'Preview' },
      { label: null, text: 'Edit' },
    ],
    previewPadding: '0px',
    previewBorderWidth: '0px',
    previewSelected: 'true',
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

it('keeps a queued draft readable while disabling editing and review actions', async () => {
  await page.viewport(390, 844)
  const summary = {
    id: 'fixture-queued-draft',
    kind: 'draft' as const,
    target: { path: 'docs/retry-plan.md' },
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
