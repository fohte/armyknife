import { expect, it } from 'vitest'

import { isReviewDocumentSummaryList } from '#components/review-document-summary'

it('accepts a review document summary list with an editor document', () => {
  expect(
    isReviewDocumentSummaryList([
      {
        id: 'fixture-review-document',
        kind: 'issue_comment',
        target: { issueNumber: 1234, title: 'Refresh the sample service' },
        isEditor: true,
        canSave: true,
        canFinish: true,
      },
    ]),
  ).toEqual(true)
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
    name: 'a non-boolean editor flag',
    item: {
      id: 'fixture-review-document',
      kind: 'issue_comment',
      target: {},
      isEditor: 'true',
    },
  },
])('rejects a review document summary with $name', ({ item }) => {
  expect(isReviewDocumentSummaryList([item])).toEqual(false)
})
