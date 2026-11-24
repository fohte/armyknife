import { expect, it } from 'vitest'

import { frontmatterForReviewFinish } from '#components/review-document-operation'

it.each([
  {
    name: 'marks a Japanese PR draft ready for translation',
    frontmatter: {
      submit: true,
      steps: { 'ready-for-translation': false, submit: true },
    },
    translate: true,
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
    expected: {
      submit: false,
      steps: { 'ready-for-translation': false, submit: false },
    },
  },
])('$name', ({ frontmatter, translate, expected }) => {
  expect(frontmatterForReviewFinish(frontmatter, translate)).toEqual(expected)
})
