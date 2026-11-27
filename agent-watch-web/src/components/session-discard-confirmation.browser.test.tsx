import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { SessionDiscardConfirmation } from '#components/session-discard-confirmation'

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

function renderConfirmation(props: {
  changes: { status: 'M'; path: string; isNew?: boolean }[]
  commits?: { sha: string; subject: string; isNew?: boolean }[]
  onCancel?: () => void
  onDiscard?: () => void
}) {
  act(() => {
    root?.render(
      <SessionDiscardConfirmation
        sessionName="Update the sample workflow"
        branchName="feature/sample-update"
        worktreePath="/workspace/sample"
        changes={props.changes}
        commits={props.commits ?? []}
        onCancel={props.onCancel ?? (() => undefined)}
        onDiscard={props.onDiscard ?? (() => undefined)}
      />,
    )
  })
}

it('expands the file list when Show more is activated', async () => {
  const changes = Array.from({ length: 7 }, (_, index) => ({
    status: 'M' as const,
    path: `src/module-${String(index + 1)}.ts`,
  }))
  renderConfirmation({ changes })

  const showMoreButton = document.querySelector<HTMLButtonElement>(
    '.session-discard-confirmation__show-more',
  )
  if (showMoreButton === null) throw new Error('Expected a Show more button')
  const initialLabel = showMoreButton.textContent
  const initialPaths = Array.from(
    document.querySelectorAll('.session-discard-confirmation__path'),
    (element) => element.textContent,
  )
  const initialExpanded = showMoreButton.getAttribute('aria-expanded')
  await act(async () => userEvent.click(showMoreButton))

  const snapshot = () => ({
    initialLabel,
    initialPaths,
    initialExpanded,
    paths: Array.from(
      document.querySelectorAll('.session-discard-confirmation__path'),
      (element) => element.textContent,
    ),
    button: document.querySelector('.session-discard-confirmation__show-more')
      ?.textContent,
    expanded: document
      .querySelector('.session-discard-confirmation__show-more')
      ?.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    initialLabel: 'Show 2 more',
    initialPaths: [
      'src/module-1.ts',
      'src/module-2.ts',
      'src/module-3.ts',
      'src/module-4.ts',
      'src/module-5.ts',
    ],
    initialExpanded: 'false',
    paths: [
      'src/module-1.ts',
      'src/module-2.ts',
      'src/module-3.ts',
      'src/module-4.ts',
      'src/module-5.ts',
      'src/module-6.ts',
      'src/module-7.ts',
    ],
    button: 'Show less',
    expanded: 'true',
  })
})

it('expands the commit list when its Show more button is activated', async () => {
  const commits = Array.from({ length: 7 }, (_, index) => ({
    sha: `a${String(index + 1)}b2c3d`,
    subject: `Update module ${String(index + 1)}`,
  }))
  renderConfirmation({ changes: [], commits })

  const showMoreButton = document.querySelector<HTMLButtonElement>(
    '.session-discard-confirmation__show-more',
  )
  if (showMoreButton === null) throw new Error('Expected a Show more button')
  const initialLabel = showMoreButton.textContent
  const initialSubjects = Array.from(
    document.querySelectorAll('.session-discard-confirmation__subject'),
    (element) => element.textContent,
  )
  const initialExpanded = showMoreButton.getAttribute('aria-expanded')
  await act(async () => userEvent.click(showMoreButton))

  const snapshot = () => ({
    initialLabel,
    initialSubjects,
    initialExpanded,
    subjects: Array.from(
      document.querySelectorAll('.session-discard-confirmation__subject'),
      (element) => element.textContent,
    ),
    button: document.querySelector('.session-discard-confirmation__show-more')
      ?.textContent,
    expanded: document
      .querySelector('.session-discard-confirmation__show-more')
      ?.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    initialLabel: 'Show 2 more',
    initialSubjects: [
      'Update module 1',
      'Update module 2',
      'Update module 3',
      'Update module 4',
      'Update module 5',
    ],
    initialExpanded: 'false',
    subjects: [
      'Update module 1',
      'Update module 2',
      'Update module 3',
      'Update module 4',
      'Update module 5',
      'Update module 6',
      'Update module 7',
    ],
    button: 'Show less',
    expanded: 'true',
  })
})

it('calls the matching action callback for each dialog button', async () => {
  const onCancel = vi.fn()
  const onDiscard = vi.fn()
  renderConfirmation({ changes: [], onCancel, onDiscard })

  const cancelButton = Array.from(document.querySelectorAll('button')).find(
    (button) => button.textContent === 'Cancel',
  )
  const discardButton = Array.from(document.querySelectorAll('button')).find(
    (button) => button.textContent === 'Discard and close',
  )
  if (cancelButton === undefined || discardButton === undefined) {
    throw new Error('Expected both dialog actions')
  }

  await userEvent.click(cancelButton)
  await userEvent.click(discardButton)

  const snapshot = () => ({
    cancelCalls: onCancel.mock.calls.length,
    discardCalls: onDiscard.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    cancelCalls: 1,
    discardCalls: 1,
  })
})
