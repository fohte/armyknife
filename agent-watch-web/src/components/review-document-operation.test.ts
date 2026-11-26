import { ok } from 'neverthrow'
import { expect, it } from 'vitest'

import { reviewDocumentActionLabel } from '#components/review-document-action-label'
import {
  frontmatterForReviewFinish,
  requestReviewDocumentAction,
} from '#components/review-document-operation'
import type { useSessionOperations } from '#session-operation-context'
import type { SessionOperation } from '#session-operation-types'

it('uses the existing PR number in the direct action label', () => {
  const labels = () => ({
    create: reviewDocumentActionLabel('create_pr'),
    update: reviewDocumentActionLabel('create_pr', {
      target: { pullRequestNumber: 73 },
    }),
    unavailable: reviewDocumentActionLabel('create_pr', {
      target: { pullRequestLookupUnavailable: true },
    }),
  })
  expect(labels()).toEqual({
    create: 'Create PR',
    update: 'Update PR #73',
    unavailable: 'Submit PR',
  })
})

it('sends the displayed PR target with its update action', () => {
  const operations: SessionOperation[] = []
  const run: ReturnType<typeof useSessionOperations>['run'] = (operation) => {
    operations.push(operation)
    return Promise.resolve(
      ok({ success: true, revision: 'next-fixture-revision' }),
    )
  }

  requestReviewDocumentAction(
    'fixture-session',
    {
      id: 'fixture-review',
      revision: 'fixture-revision',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
    },
    {
      id: 'fixture-review',
      kind: 'pr_draft',
      target: { pullRequestNumber: 318 },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    'create_pr',
    () => undefined,
    run,
    'Fixture session',
    null,
  )

  expect(operations).toEqual([
    {
      type: 'review_document_action',
      sessionId: 'fixture-session',
      id: 'fixture-review',
      revision: 'fixture-revision',
      frontmatter: { title: 'Fixture title' },
      body: 'Fixture body',
      action: 'create_pr',
      expectedPullRequestTarget: { kind: 'existing', number: 318 },
    },
  ])
})

it.each([
  {
    name: 'marks a Japanese PR draft ready for translation',
    frontmatter: {
      submit: true,
      steps: { 'ready-for-translation': false, submit: true },
    },
    translate: true,
    preserveSubmit: false,
    expected: {
      submit: false,
      steps: { 'ready-for-translation': true, submit: false },
    },
  },
  {
    name: 'clears approval flags when sending a draft back',
    frontmatter: {
      submit: true,
      steps: { 'ready-for-translation': true, submit: true },
    },
    translate: false,
    preserveSubmit: false,
    expected: {
      submit: false,
      steps: { 'ready-for-translation': false, submit: false },
    },
  },
  {
    name: 'preserves issue approval when finishing a new issue draft',
    frontmatter: { title: 'Fixture issue title', submit: true },
    translate: false,
    preserveSubmit: true,
    expected: { title: 'Fixture issue title', submit: true },
  },
])('$name', ({ frontmatter, translate, preserveSubmit, expected }) => {
  expect(
    frontmatterForReviewFinish(frontmatter, translate, preserveSubmit),
  ).toEqual(expected)
})
