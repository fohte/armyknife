import { expect, it } from 'vitest'

import {
  parseReviewThreads,
  updateReviewThread,
} from '#components/review-document-preview-data'

it('preserves trailing whitespace in an edited reply', () => {
  const body = `<!-- thread: THREAD_reply path: src/example.ts:11 author: @sample-reviewer -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -10,1 +10,1 @@
> context line
\`\`\`
<!-- /diff -->
<!-- comment: @sample-reviewer 2026-11-24T00:00:00Z -->
Please revise.
<!-- /comment -->`
  const updated = updateReviewThread(body, 'THREAD_reply', {
    reply: 'Keep both lines. \n',
  })

  expect(parseReviewThreads(updated)).toEqual([
    {
      id: 'THREAD_reply',
      path: 'src/example.ts',
      line: 11,
      resolved: false,
      diff: [
        {
          text: '@@ -10,1 +10,1 @@',
          kind: 'marker',
          focused: false,
        },
        { text: ' context line', kind: 'context', focused: true },
      ],
      comments: [
        {
          author: '@sample-reviewer',
          createdAt: '2026-11-24T00:00:00Z',
          body: 'Please revise.',
        },
      ],
      reply: 'Keep both lines. \n',
    },
  ])
})

it('keeps an empty reviewer comment separate from the next comment', () => {
  const body = `<!-- thread: THREAD_comments path: src/example.ts:11 author: @first-reviewer -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -10,1 +10,1 @@
> context line
\`\`\`
<!-- /diff -->
<!-- comment: @first-reviewer 2026-11-24T00:00:00Z -->
<!-- /comment -->

<!-- comment: @second-reviewer 2026-11-24T00:01:00Z -->
Please clarify the behavior.
<!-- /comment -->`

  expect(parseReviewThreads(body)).toEqual([
    {
      id: 'THREAD_comments',
      path: 'src/example.ts',
      line: 11,
      resolved: false,
      diff: [
        {
          text: '@@ -10,1 +10,1 @@',
          kind: 'marker',
          focused: false,
        },
        { text: ' context line', kind: 'context', focused: true },
      ],
      comments: [
        {
          author: '@first-reviewer',
          createdAt: '2026-11-24T00:00:00Z',
          body: '',
        },
        {
          author: '@second-reviewer',
          createdAt: '2026-11-24T00:01:00Z',
          body: 'Please clarify the behavior.',
        },
      ],
      reply: '',
    },
  ])
})

it('highlights the target line in an older compressed diff', () => {
  const body = `<!-- thread: THREAD_legacy path: src/legacy.ts:11 author: @sample-reviewer -->
- [ ] resolve
<!-- diff -->
\`\`\`diff
@@ -1,20 +1,20 @@
... [5 lines omitted] ...
 line 6
 line 7
 line 8
 line 9
 line 10
 line 11
 line 12
\`\`\`
<!-- /diff -->
<!-- comment: @sample-reviewer 2026-11-24T00:00:00Z -->
Please update this line.
<!-- /comment -->`

  expect(parseReviewThreads(body)).toEqual([
    {
      id: 'THREAD_legacy',
      path: 'src/legacy.ts',
      line: 11,
      resolved: false,
      diff: [
        {
          text: '@@ -1,20 +1,20 @@',
          kind: 'marker',
          focused: false,
        },
        {
          text: '... [5 lines omitted] ...',
          kind: 'marker',
          focused: false,
        },
        { text: ' line 6', kind: 'context', focused: false },
        { text: ' line 7', kind: 'context', focused: false },
        { text: ' line 8', kind: 'context', focused: false },
        { text: ' line 9', kind: 'context', focused: false },
        { text: ' line 10', kind: 'context', focused: false },
        { text: ' line 11', kind: 'context', focused: true },
        { text: ' line 12', kind: 'context', focused: false },
      ],
      comments: [
        {
          author: '@sample-reviewer',
          createdAt: '2026-11-24T00:00:00Z',
          body: 'Please update this line.',
        },
      ],
      reply: '',
    },
  ])
})
