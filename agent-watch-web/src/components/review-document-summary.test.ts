import { expect, it } from 'vitest'

import { isReviewDocumentSummaryList } from '#components/review-document-summary'

it('accepts a review document summary list with an editor document', () => {
  expect(
    isReviewDocumentSummaryList([
      {
        id: 'fixture-review-document',
        kind: 'draft',
        target: { path: 'docs/retry-plan.md' },
        startedAt: Date.now() - 180_000,
        isEditor: true,
        canSave: true,
        canFinish: true,
      },
    ]),
  ).toEqual(true)
})

it('accepts an unavailable pull request lookup on a draft summary', () => {
  const summary = () =>
    isReviewDocumentSummaryList([
      {
        id: 'fixture-review-document',
        kind: 'pr_draft',
        target: {
          repository: 'sample-owner/sample-repository',
          branch: 'feature/example',
          pullRequestLookupUnavailable: true,
        },
        isEditor: true,
        canSave: true,
        canFinish: true,
      },
    ])

  expect(summary()).toEqual(true)
})

it.each([
  {
    name: 'an unknown document kind',
    item: {
      id: 'fixture-review-document',
      kind: 'issue_discussion',
      target: {},
      isEditor: true,
    },
  },
  {
    name: 'a missing target',
    item: {
      id: 'fixture-review-document',
      kind: 'issue_comment',
      isEditor: true,
    },
  },
  {
    name: 'a non-string draft path',
    item: {
      id: 'fixture-review-document',
      kind: 'draft',
      target: { path: 1234 },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
  },
  {
    name: 'a non-boolean editor flag',
    item: {
      id: 'fixture-review-document',
      kind: 'issue_comment',
      target: {},
      isEditor: 'true',
    },
  },
  {
    name: 'an unsafe start timestamp',
    item: {
      id: 'fixture-review-document',
      kind: 'issue_comment',
      target: {},
      startedAt: Number.MAX_SAFE_INTEGER + 1,
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
  },
])('rejects a review document summary with $name', ({ item }) => {
  expect(isReviewDocumentSummaryList([item])).toEqual(false)
})
