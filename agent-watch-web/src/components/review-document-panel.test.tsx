import '#index.css'

import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page, userEvent } from 'vitest/browser'

import { ReviewDocumentPanel } from '#components/review-document-panel'
import type { ReviewDocumentSummary } from '#components/review-document-summary'
import { SessionStatusPanel } from '#components/session-status-panel'
import type { JsonValue } from '#types'

let container: HTMLDivElement
let root: Root | null = null
const actionSnapshots: {
  action: 'save' | 'finish'
  frontmatter: JsonValue
  body: string
}[] = []
const directActions: string[] = []

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
  directActions.length = 0
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

it('posts an issue comment through its action button and preserves Send back', async () => {
  const summary = {
    id: 'fixture-issue-comment',
    kind: 'issue_comment' as const,
    target: { repository: 'sample-owner/sample-repository', issueNumber: 42 },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={summary}
        frontmatter={{ submit: false }}
        body="Fixture comment"
        isMobile
        onSubmit={(action) => directActions.push(action)}
        onFinish={() => directActions.push('send_back')}
      />,
    ),
  )

  await act(async () => {
    await page.getByRole('button', { name: 'Post comment' }).click()
  })
  const snapshot = () => ({
    buttons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.review-document-direct-button-row button',
      ),
      (button) => ({
        label: button.textContent,
        disabled: button.disabled,
        primary: button.classList.contains('review-document-finish'),
      }),
    ),
    metadata: container.querySelector('.review-document-frontmatter'),
    help: container.querySelector('.review-document-direct-help')?.textContent,
    actions: directActions,
  })
  expect(snapshot()).toEqual({
    buttons: [
      { label: 'Send back', disabled: false, primary: false },
      { label: 'Post comment', disabled: false, primary: true },
    ],
    metadata: null,
    help: 'Post comment posts it to #42 right away. Send back returns your edits to the session.',
    actions: ['post_comment'],
  })
})

it('shows a save conflict error in the desktop direct action footer', () => {
  root = createRoot(container)
  act(() =>
    root?.render(
      <div
        className="session-status-panel"
        data-size="desktop"
        style={{ width: '640px', height: '900px', display: 'flex' }}
      >
        <ReviewDocumentPanel
          summary={{
            id: 'fixture-issue-comment',
            kind: 'issue_comment',
            target: {
              repository: 'sample-owner/sample-repository',
              issueNumber: 73,
            },
            isEditor: true,
            canSave: true,
            canFinish: true,
          }}
          frontmatter={{ submit: false }}
          body="Example draft body"
          status="error"
          error="This draft changed since you opened it. Your edits are still here; copy them before reloading to see the latest version."
        />
      </div>,
    ),
  )

  const saveConflict = container.querySelector(
    '.review-document-direct-actions [role="alert"]',
  )
  expect(saveConflict?.textContent ?? null).toEqual(
    'This draft changed since you opened it. Your edits are still here; copy them before reloading to see the latest version.',
  )
})

it.each([
  {
    name: 'review replies',
    kind: 'review_replies' as const,
    target: { pullRequestNumber: 42, threadCount: 2 },
    label: 'Post replies',
    action: 'post_replies',
  },
  {
    name: 'an issue body',
    kind: 'issue_body' as const,
    target: { repository: 'sample-owner/sample-repository', issueNumber: 42 },
    label: 'Update issue',
    action: 'update_issue',
  },
])('submits the selected action for $name', async (testCase) => {
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={{
          id: 'fixture-review-document',
          kind: testCase.kind,
          target: testCase.target,
          isEditor: true,
          canSave: true,
          canFinish: true,
        }}
        frontmatter={{ submit: false }}
        body="Example draft body"
        isMobile
        onSubmit={(action) => directActions.push(action)}
      />,
    ),
  )

  await act(async () => {
    await page.getByRole('button', { name: testCase.label }).click()
  })

  expect(directActions).toEqual([testCase.action])
})

it('routes the English-repo PR draft through translation before creation', async () => {
  const summary = {
    id: 'fixture-pr-draft',
    kind: 'pr_draft' as const,
    target: {
      repository: 'sample-owner/sample-repository',
      branch: 'feature/example',
    },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  const onApproveForTranslation = vi.fn(() =>
    directActions.push('approve_for_translation'),
  )
  const onSubmit = vi.fn((action: string) => directActions.push(action))
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={summary}
        frontmatter={{
          title: '再送の上限を変える',
          steps: { 'ready-for-translation': false, submit: false },
        }}
        body="日本語の PR draft"
        onApproveForTranslation={onApproveForTranslation}
        onSubmit={onSubmit}
      />,
    ),
  )

  await act(async () => {
    await page.getByRole('button', { name: 'Approve for translation' }).click()
  })
  const snapshot = () => ({
    stages: Array.from(
      container.querySelectorAll('.review-document-pr-stages li'),
      (stage) => ({
        label: stage.textContent,
        current: stage.getAttribute('aria-current'),
      }),
    ),
    primary: container.querySelector('.review-document-finish')?.textContent,
    callbacks: {
      translation: onApproveForTranslation.mock.calls.length,
      submit: onSubmit.mock.calls.length,
      actions: directActions,
    },
  })
  expect(snapshot()).toEqual({
    stages: [
      { label: '1Review—', current: 'step' },
      { label: '2Translate—', current: 'false' },
      { label: '3Create PR—', current: 'false' },
    ],
    primary: 'Approve for translation',
    callbacks: {
      translation: 1,
      submit: 0,
      actions: ['approve_for_translation'],
    },
  })
})

it('shows Create PR at the final stage after an English translation', async () => {
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={{
          id: 'fixture-pr-draft',
          kind: 'pr_draft',
          target: {
            repository: 'sample-owner/sample-repository',
            branch: 'feature/example',
          },
          isEditor: true,
          canSave: true,
          canFinish: true,
        }}
        frontmatter={{
          title: 'Make the retry limit configurable',
          steps: { 'ready-for-translation': false, submit: false },
        }}
        body="The retry limit is configurable."
        onSubmit={(action) => directActions.push(action)}
      />,
    ),
  )

  await act(async () => {
    await page.getByRole('button', { name: 'Create PR' }).click()
  })
  const snapshot = () => ({
    stageLabels: Array.from(
      container.querySelectorAll('.review-document-pr-stages li'),
      (stage) => stage.textContent,
    ),
    primary: container.querySelector('.review-document-finish')?.textContent,
    actions: directActions,
  })
  expect(snapshot()).toEqual({
    stageLabels: ['✓Review—', '✓Translate—', '3Create PR—'],
    primary: 'Create PR',
    actions: ['create_pr'],
  })
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
    await page.getByRole('button', { name: 'Finish' }).click()
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

it('shows issue fields as typed metadata before the Markdown body', () => {
  const summary = {
    id: 'fixture-issue-body',
    kind: 'issue_body' as const,
    target: {
      repository: 'example/web-app',
      issueNumber: 412,
      title: 'Delivery checks fail on the fourth attempt',
    },
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
        frontmatter={{
          title: 'Notifications stop after several deliveries',
          labels: ['bug', 'notifications'],
          assignees: ['sample-maintainer'],
          parentIssue: 'example/web-app#390',
          subIssues: ['example/web-app#413', 'example/web-app#414'],
          submit: false,
          readonly: { number: 412, author: 'sample-reporter' },
        }}
        body={`## Summary

The fourth notification is not delivered.`}
      />,
    ),
  )

  const fields = container.querySelector('.review-document-issue-fields')
  const fieldRow = (name: string) => {
    const label = [...(fields?.querySelectorAll('dt') ?? [])].find(
      (item) => item.textContent === name,
    )
    return label?.nextElementSibling
  }
  const parentRow = fieldRow('Parent issue')
  const parentLink = parentRow?.querySelector('a')
  const subIssueLinks = [
    ...(fields?.querySelectorAll(
      '.review-document-issue-sub-items .review-document-issue-reference-link',
    ) ?? []),
  ]
  const snapshot = () => ({
    title: container.querySelector<HTMLTextAreaElement>(
      'textarea[aria-label="Issue title"]',
    )?.value,
    labels: [...(fields?.querySelectorAll('.review-document-issue-chip') ?? [])]
      .filter(
        (item) =>
          !item.classList.contains('review-document-issue-chip--assignee'),
      )
      .map((item) => item.textContent.replaceAll('×', '').trim()),
    assignees: [
      ...(fields?.querySelectorAll('.review-document-issue-chip--assignee') ??
        []),
    ].map((item) => ({
      initial: item.querySelector('span')?.textContent,
      name: item.textContent
        .replace(item.querySelector('span')?.textContent ?? '', '')
        .replaceAll('×', '')
        .trim(),
    })),
    parentIssue: parentLink?.textContent,
    parentIssueHref: parentLink?.getAttribute('href'),
    parentIssueTarget: parentLink?.getAttribute('target'),
    parentIssueRel: parentLink?.getAttribute('rel'),
    setParentButton: parentRow?.querySelector(
      'button[aria-label="Set parent issue"]',
    )?.textContent,
    subIssues: subIssueLinks.map((item) => ({
      reference: item.textContent,
      href: item.getAttribute('href'),
      target: item.getAttribute('target'),
      rel: item.getAttribute('rel'),
    })),
    submit: container.querySelector('input[aria-label="submit"]') !== null,
    body: container.querySelector('.review-document-body')?.textContent,
  })

  expect(snapshot()).toEqual({
    title: 'Notifications stop after several deliveries',
    labels: ['bug', 'notifications'],
    assignees: [{ initial: 'S', name: 'sample-maintainer' }],
    parentIssue: 'example/web-app#390',
    parentIssueHref: 'https://github.com/example/web-app/issues/390',
    parentIssueTarget: '_blank',
    parentIssueRel: 'noopener noreferrer',
    setParentButton: 'Set parent',
    subIssues: [
      {
        reference: 'example/web-app#413',
        href: 'https://github.com/example/web-app/issues/413',
        target: '_blank',
        rel: 'noopener noreferrer',
      },
      {
        reference: 'example/web-app#414',
        href: 'https://github.com/example/web-app/issues/414',
        target: '_blank',
        rel: 'noopener noreferrer',
      },
    ],
    submit: false,
    body: `Summary
The fourth notification is not delivered.`,
  })
})

it('keeps malformed issue references visible as plain text', () => {
  const summary = {
    id: 'fixture-issue-body-malformed-references',
    kind: 'issue_body' as const,
    target: {
      repository: 'example/web-app',
      issueNumber: 412,
      title: 'Delivery checks fail on the fourth attempt',
    },
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
        frontmatter={{
          title: 'Notifications stop after several deliveries',
          labels: [],
          assignees: [],
          parentIssue: 'example/web-app#abc',
          subIssues: ['malformed reference'],
          submit: false,
          readonly: { number: 412, author: 'sample-reporter' },
        }}
        body="Issue description."
      />,
    ),
  )

  const fields = container.querySelector('.review-document-issue-fields')
  const fieldRow = (name: string) => {
    const label = [...(fields?.querySelectorAll('dt') ?? [])].find(
      (item) => item.textContent === name,
    )
    return label?.nextElementSibling
  }
  const parentRow = fieldRow('Parent issue')
  const parentReference = parentRow?.querySelector(
    '.review-document-issue-reference',
  )
  const subIssueReferences = [
    ...(fields?.querySelectorAll(
      '.review-document-issue-sub-items .review-document-issue-reference',
    ) ?? []),
  ]

  const snapshot = () => ({
    parentReference: {
      text: parentReference?.textContent.replace('×', '').trim(),
      tag: parentReference?.querySelector('code')?.tagName,
      link: parentReference?.querySelector('a')?.getAttribute('href'),
    },
    setParentButton: parentRow?.querySelector(
      'button[aria-label="Set parent issue"]',
    )?.textContent,
    subIssues: subIssueReferences.map((item) => ({
      text: item.textContent.replace('×', '').trim(),
      tag: item.querySelector('code')?.tagName,
      link: item.querySelector('a')?.getAttribute('href'),
    })),
  })

  expect(snapshot()).toEqual({
    parentReference: {
      text: 'example/web-app#abc',
      tag: 'CODE',
      link: undefined,
    },
    setParentButton: 'Set parent',
    subIssues: [
      {
        text: 'malformed reference',
        tag: 'CODE',
        link: undefined,
      },
    ],
  })
})

it('saves pending issue metadata when Save is clicked', async () => {
  const savedFrontmatters: JsonValue[] = []
  const summary = {
    id: 'fixture-issue-body-pending-metadata',
    kind: 'issue_body' as const,
    target: {
      repository: 'example/web-app',
      issueNumber: 412,
    },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }

  function Harness() {
    const [frontmatter, setFrontmatter] = useState<JsonValue>({
      title: 'Fixture issue',
      labels: [],
      assignees: [],
      parentIssue: '',
      subIssues: [],
      submit: false,
      readonly: { number: 412, author: 'sample-reporter' },
    })
    const [body, setBody] = useState('Fixture issue body.')

    return (
      <ReviewDocumentPanel
        summary={summary}
        summaries={[summary]}
        frontmatter={frontmatter}
        body={body}
        onFrontmatterChange={setFrontmatter}
        onBodyChange={setBody}
        onSave={() => {
          savedFrontmatters.push(frontmatter)
        }}
      />
    )
  }

  await page.viewport(640, 900)
  root = createRoot(container)
  act(() => root?.render(<Harness />))
  await act(async () => {
    await page.getByRole('textbox', { name: 'Add label' }).fill('notifications')
    await page.getByRole('button', { name: 'Add assignee' }).click()
    await page
      .getByRole('textbox', { name: 'Add assignee' })
      .fill('sample-maintainer')
    await page.getByRole('button', { name: 'Add sub-issue' }).click()
    await page
      .getByRole('textbox', { name: 'Add sub-issue' })
      .fill('example/web-app#415')
    await page.getByRole('button', { name: 'Set parent issue' }).click()
    await page
      .getByRole('textbox', { name: 'Set parent issue' })
      .fill('example/web-app#390')
    await page.getByRole('button', { name: 'Save' }).click()
  })

  expect(savedFrontmatters).toEqual([
    {
      title: 'Fixture issue',
      labels: ['notifications'],
      assignees: ['sample-maintainer'],
      parentIssue: 'example/web-app#390',
      subIssues: ['example/web-app#415'],
      submit: false,
      readonly: { number: 412, author: 'sample-reporter' },
    },
  ])
})

it('keeps the issue milestone available in Edit mode', () => {
  const summary = {
    id: 'fixture-issue-body-milestone',
    kind: 'issue_body' as const,
    target: { repository: 'example/web-app', issueNumber: 412 },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  const frontmatter = {
    title: 'Fixture issue',
    labels: [],
    assignees: [],
    parentIssue: '',
    subIssues: [],
    milestone: 'Quarterly maintenance',
    submit: false,
    readonly: { number: 412, author: 'sample-reporter' },
  }
  const renderPanel = (view: 'preview' | 'edit') => (
    <ReviewDocumentPanel
      summary={summary}
      summaries={[summary]}
      frontmatter={frontmatter}
      body="Fixture issue body."
      view={view}
    />
  )

  root = createRoot(container)
  act(() => root?.render(renderPanel('preview')))
  const previewFields = [
    ...container.querySelectorAll(
      '.review-document-fields .review-document-field-key',
    ),
  ].map((item) => item.textContent)
  act(() => root?.render(renderPanel('edit')))

  const snapshot = () => ({
    previewFields,
    editFields: [
      ...container.querySelectorAll(
        '.review-document-fields .review-document-field-key',
      ),
    ].map((item) => item.textContent),
    milestone: container.querySelector<HTMLInputElement>(
      '.review-document-fields input[aria-label="milestone"]',
    )?.value,
  })

  expect(snapshot()).toEqual({
    previewFields: [],
    editFields: ['milestone', 'submit'],
    milestone: 'Quarterly maintenance',
  })
})

const approvalDocumentCases: {
  name: string
  summary: ReviewDocumentSummary
  frontmatter: { [key: string]: JsonValue }
}[] = [
  {
    name: 'issue body',
    summary: {
      id: 'fixture-issue-body',
      kind: 'issue_body',
      target: { repository: 'example/mock-repo', issueNumber: 89 },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    frontmatter: {
      title: 'Fixture issue',
      labels: [],
      assignees: [],
      parentIssue: '',
      subIssues: [],
      submit: false,
      readonly: { number: 89, author: 'sample-reporter' },
    },
  },
  {
    name: 'review replies',
    summary: {
      id: 'fixture-review-replies',
      kind: 'review_replies',
      target: {
        repository: 'example/mock-repo',
        pullRequestNumber: 21,
        threadCount: 1,
      },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    frontmatter: { submit: false },
  },
]

it.each(approvalDocumentCases)(
  'keeps $name approval editable in Edit',
  async ({ summary, frontmatter }) => {
    const savedFrontmatter: JsonValue[] = []

    function Harness() {
      const [currentFrontmatter, setFrontmatter] =
        useState<JsonValue>(frontmatter)
      return (
        <ReviewDocumentPanel
          summary={summary}
          view="edit"
          frontmatter={currentFrontmatter}
          body="Fixture body."
          onFrontmatterChange={setFrontmatter}
          onSave={() => savedFrontmatter.push(currentFrontmatter)}
        />
      )
    }

    root = createRoot(container)
    act(() => root?.render(<Harness />))

    await act(async () => {
      await page.getByRole('checkbox', { name: 'submit' }).click()
    })
    await act(async () => {
      await page.getByRole('button', { name: 'Save' }).click()
    })

    expect(savedFrontmatter).toEqual([{ ...frontmatter, submit: true }])
  },
)

it('adds issue metadata values and saves the full frontmatter', async () => {
  const savedFrontmatter: JsonValue[] = []
  const initialFrontmatter: JsonValue = {
    title: 'Fixture issue',
    labels: ['existing-label'],
    assignees: [],
    parentIssue: '',
    subIssues: [],
    submit: false,
    readonly: { number: 89, author: 'sample-reporter' },
  }
  const summary = {
    id: 'fixture-issue-body',
    kind: 'issue_body' as const,
    target: { repository: 'example/mock-repo', issueNumber: 89 },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }

  function Harness() {
    const [frontmatter, setFrontmatter] = useState(initialFrontmatter)
    return (
      <ReviewDocumentPanel
        summary={summary}
        frontmatter={frontmatter}
        body="Fixture body."
        onFrontmatterChange={setFrontmatter}
        onSave={() => savedFrontmatter.push(frontmatter)}
      />
    )
  }

  root = createRoot(container)
  act(() => root?.render(<Harness />))

  await act(async () => {
    const label = page.getByRole('textbox', { name: 'Add label' })
    await label.fill('priority-label')
    await userEvent.keyboard('{Enter}')
    await page.getByRole('button', { name: 'Add assignee' }).click()
    const assignee = page.getByRole('textbox', { name: 'Add assignee' })
    await assignee.fill('sample-owner')
    await userEvent.keyboard('{Enter}')
    await page.getByRole('button', { name: 'Set parent issue' }).click()
    const parent = page.getByRole('textbox', { name: 'Set parent issue' })
    await parent.fill('example/mock-repo#80')
    await userEvent.keyboard('{Enter}')
    await page.getByRole('button', { name: 'Add sub-issue' }).click()
    const subIssue = page.getByRole('textbox', { name: 'Add sub-issue' })
    await subIssue.fill('example/mock-repo#90')
    await userEvent.keyboard('{Enter}')
  })
  await act(async () => {
    await page.getByRole('button', { name: 'Save' }).click()
  })

  expect(savedFrontmatter).toEqual([
    {
      title: 'Fixture issue',
      labels: ['existing-label', 'priority-label'],
      assignees: ['sample-owner'],
      parentIssue: 'example/mock-repo#80',
      subIssues: ['example/mock-repo#90'],
      submit: false,
      readonly: { number: 89, author: 'sample-reporter' },
    },
  ])
})

it('updates parent issue using the issue frontmatter camelCase key', async () => {
  const summary = {
    id: 'fixture-issue-body',
    kind: 'issue_body' as const,
    target: { repository: 'example/web-app', issueNumber: 412 },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  const frontmatter: JsonValue = {
    title: 'Fixture title',
    labels: ['bug'],
    assignees: ['sample-maintainer'],
    parentIssue: 'example/web-app#390',
    subIssues: ['example/web-app#413'],
    submit: false,
    readonly: { number: 412, author: 'sample-reporter' },
  }
  const changes: JsonValue[] = []
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={summary}
        frontmatter={frontmatter}
        body="Fixture body."
        onFrontmatterChange={(value) => changes.push(value)}
      />,
    ),
  )

  await act(async () => {
    await page.getByRole('button', { name: 'Clear parent issue' }).click()
  })

  expect(changes).toEqual([
    {
      title: 'Fixture title',
      labels: ['bug'],
      assignees: ['sample-maintainer'],
      parentIssue: null,
      subIssues: ['example/web-app#413'],
      submit: false,
      readonly: { number: 412, author: 'sample-reporter' },
    },
  ])
})

it('shows an issue comment body without its file metadata', () => {
  const summary = {
    id: 'fixture-issue-comment',
    kind: 'issue_comment' as const,
    target: { repository: 'example/web-app', issueNumber: 412 },
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
        frontmatter={null}
        body={
          '<!-- author: sample-reviewer -->\n<!-- createdAt: 2026-11-24T02:40:00Z -->\n<!-- updatedAt: 2026-11-24T02:40:00Z -->\n<!-- id: comment-node -->\n<!-- databaseId: 506 -->\n\nCould we retry the pending delivery after a short delay?'
        }
      />,
    ),
  )

  const snapshot = () => ({
    metadataSections: container.querySelectorAll('.review-document-frontmatter')
      .length,
    commentHeaders: container.querySelectorAll(
      '.review-document-comment-header',
    ).length,
    timestamps: container.querySelectorAll('time').length,
    message: container.querySelector('.review-document-issue-comment-body')
      ?.textContent,
    rawMetadataVisible: container
      .querySelector('.review-document-preview-column')
      ?.textContent.includes('databaseId'),
  })

  expect(snapshot()).toEqual({
    metadataSections: 0,
    commentHeaders: 0,
    timestamps: 0,
    message: 'Could we retry the pending delivery after a short delay?',
    rawMetadataVisible: false,
  })
})

it('shows each review thread with its focused diff, reviewer comment, and actions', () => {
  const summary = {
    id: 'fixture-review-replies',
    kind: 'review_replies' as const,
    target: {
      repository: 'example/web-app',
      pullRequestNumber: 2049,
      threadCount: 2,
    },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  const body = `<!-- thread: THREAD_sample_1 path: src/notifications/delivery.rs:15 author: @sample-reviewer -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -14,5 +14,6 @@
 fn deliver() {
-    send();
>+    retry_pending();
+    send();
 }
\`\`\`
<!-- /diff -->
<!-- comment: @sample-reviewer 2026-11-23T18:30:00Z -->
Please keep the original delivery attempt after retrying.
<!-- /comment -->
Retain the first send after the retry.

<!-- thread: THREAD_sample_2 path: src/notifications/retry.rs:40 author: @another-reviewer -->
- [x] resolve
<!-- diff -->
\`\`\`diff
@@ -39,3 +39,2 @@
 fn retry() {
>-    delay();
 }
\`\`\`
<!-- /diff -->
<!-- comment: @another-reviewer 2026-11-23T18:42:00Z -->
Please keep this delay configurable.
<!-- /comment -->`
  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={summary}
        summaries={[summary]}
        frontmatter={{ submit: false }}
        body={body}
      />,
    ),
  )

  const snapshot = () => ({
    frontmatterSections: container.querySelectorAll(
      '.review-document-frontmatter',
    ).length,
    threads: [
      ...container.querySelectorAll<HTMLElement>('.review-document-thread'),
    ].map((thread) => ({
      path: thread.querySelector('.review-document-thread-path')?.textContent,
      resolved: thread.querySelector<HTMLInputElement>(
        '.review-document-thread-resolve input',
      )?.checked,
      diff: [...thread.querySelectorAll('.review-document-diff-line')].map(
        (line) => ({
          text: line.textContent,
          focused: line.classList.contains(
            'review-document-diff-line--focused',
          ),
        }),
      ),
      comments: [
        ...thread.querySelectorAll('.review-document-thread-comment-card'),
      ].map((comment) => ({
        author: comment.querySelector('strong')?.textContent,
        body: comment.querySelector('.review-document-thread-comment-body')
          ?.textContent,
      })),
      reply: thread.querySelector<HTMLTextAreaElement>(
        '.review-document-thread-reply textarea',
      )?.value,
    })),
  })

  expect(snapshot()).toEqual({
    frontmatterSections: 0,
    threads: [
      {
        path: 'src/notifications/delivery.rs:15',
        resolved: false,
        diff: [
          { text: '@@ -14,5 +14,6 @@', focused: false },
          { text: ' fn deliver() {', focused: false },
          { text: '-    send();', focused: false },
          { text: '+    retry_pending();', focused: true },
          { text: '+    send();', focused: false },
          { text: ' }', focused: false },
        ],
        comments: [
          {
            author: '@sample-reviewer',
            body: 'Please keep the original delivery attempt after retrying.',
          },
        ],
        reply: 'Retain the first send after the retry.',
      },
      {
        path: 'src/notifications/retry.rs:40',
        resolved: true,
        diff: [
          { text: '@@ -39,3 +39,2 @@', focused: false },
          { text: ' fn retry() {', focused: false },
          { text: '-    delay();', focused: true },
          { text: ' }', focused: false },
        ],
        comments: [
          {
            author: '@another-reviewer',
            body: 'Please keep this delay configurable.',
          },
        ],
        reply: '',
      },
    ],
  })
})

it('shows all PR review messages in order when there are no inline threads', () => {
  const summary = {
    id: 'fixture-review-summary',
    kind: 'review_replies' as const,
    target: {
      repository: 'example/web-app',
      pullRequestNumber: 2049,
      threadCount: 0,
    },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  const body = `<!-- reviews -->
<!-- review: @sample-reviewer state=COMMENTED 2026-11-24T00:00:00Z -->
The overall review needs a response.
<!-- /review -->
<!-- review: @another-reviewer state=COMMENTED 2026-11-24T00:01:00Z -->
A second reviewer requests a follow-up.
<!-- /review -->
<!-- /reviews -->`

  root = createRoot(container)
  act(() =>
    root?.render(
      <ReviewDocumentPanel
        summary={summary}
        summaries={[summary]}
        frontmatter={{ submit: false }}
        body={body}
      />,
    ),
  )

  const snapshot = () => ({
    threadCards: container.querySelectorAll('.review-document-thread').length,
    reviewMessages: Array.from(
      container.querySelectorAll('.review-document-body p'),
      (paragraph) => paragraph.textContent,
    ),
  })

  expect(snapshot()).toEqual({
    threadCards: 0,
    reviewMessages: [
      'The overall review needs a response.',
      'A second reviewer requests a follow-up.',
    ],
  })
})

it('updates a reply and resolve checkbox in the review threads draft', async () => {
  const savedBodies: string[] = []
  const summary = {
    id: 'fixture-review-replies',
    kind: 'review_replies' as const,
    target: {
      repository: 'example/web-app',
      pullRequestNumber: 2049,
      threadCount: 2,
    },
    isEditor: true,
    canSave: true,
    canFinish: true,
  }
  const originalBody = `<!-- thread: THREAD_fixture_1 path: src/example.rs:9 author: @sample-reviewer -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -8,3 +8,4 @@
 fn handle() {
+    retry();
 }
\`\`\`
<!-- /diff -->
<!-- comment: @sample-reviewer 2026-11-23T18:30:00Z -->
Keep this change small.
<!-- /comment -->
Retain the current behavior.

<!-- thread: THREAD_fixture_2 path: src/other.rs:20 author: @another-reviewer -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -19,3 +19,4 @@
 fn finish() {
+    close();
 }
\`\`\`
<!-- /diff -->
<!-- comment: @another-reviewer 2026-11-23T18:42:00Z -->
Please close the handle.
<!-- /comment -->`

  function Harness() {
    const [body, setBody] = useState(originalBody)
    return (
      <div
        className="session-status-panel"
        data-size="desktop"
        style={{ width: '640px', height: '900px', display: 'flex' }}
      >
        <ReviewDocumentPanel
          summary={summary}
          frontmatter={{ submit: false }}
          body={body}
          onBodyChange={(nextBody) => {
            setBody(nextBody)
          }}
          onSave={() => savedBodies.push(body)}
        />
      </div>
    )
  }

  await page.viewport(640, 900)
  root = createRoot(container)
  act(() => root?.render(<Harness />))

  await act(async () => {
    await page
      .getByRole('textbox', { name: 'Your reply to src/example.rs' })
      .fill('Use a bounded retry.\n\nKeep the limit configurable.')
  })
  await act(async () => {
    await page.getByRole('checkbox', { name: 'Resolve' }).first().click()
  })
  await act(async () => {
    await page.getByRole('button', { name: 'Save' }).click()
  })

  const expectedBody = [
    '<!-- thread: THREAD_fixture_1 path: src/example.rs:9 author: @sample-reviewer -->',
    '- [x] resolve',
    '<!-- diff -->',
    '```diff',
    '@@ -8,3 +8,4 @@',
    ' fn handle() {',
    '+    retry();',
    ' }',
    '```',
    '<!-- /diff -->',
    '<!-- comment: @sample-reviewer 2026-11-23T18:30:00Z -->',
    'Keep this change small.',
    '<!-- /comment -->',
    'Use a bounded retry.',
    '',
    'Keep the limit configurable.',
    '',
    '<!-- thread: THREAD_fixture_2 path: src/other.rs:20 author: @another-reviewer -->',
    '- [ ] resolve',
    '<!-- diff -->',
    '```diff',
    '@@ -19,3 +19,4 @@',
    ' fn finish() {',
    '+    close();',
    ' }',
    '```',
    '<!-- /diff -->',
    '<!-- comment: @another-reviewer 2026-11-23T18:42:00Z -->',
    'Please close the handle.',
    '<!-- /comment -->',
  ].join('\n')
  expect(savedBodies).toEqual([expectedBody])
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
      'review-document-actions review-document-direct-actions',
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

it('keeps the approval checkbox available for a new issue draft', async () => {
  const finishSnapshots: JsonValue[] = []
  function NewIssueDraftHarness() {
    const [frontmatter, setFrontmatter] = useState<JsonValue>({
      title: 'Fixture issue title',
      submit: false,
    })
    return (
      <ReviewDocumentPanel
        summary={{
          id: 'fixture-new-issue',
          kind: 'draft',
          target: { repository: 'sample-owner/sample-repository' },
          isEditor: true,
          canSave: true,
          canFinish: true,
        }}
        frontmatter={frontmatter}
        body="Fixture issue body"
        onFrontmatterChange={setFrontmatter}
        onFinish={() => finishSnapshots.push(frontmatter)}
      />
    )
  }

  root = createRoot(container)
  act(() => root?.render(<NewIssueDraftHarness />))

  await act(async () => {
    await page.getByRole('checkbox', { name: 'submit' }).click()
    await page.getByRole('button', { name: 'Finish' }).click()
  })

  expect(finishSnapshots).toEqual([
    { title: 'Fixture issue title', submit: true },
  ])
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
