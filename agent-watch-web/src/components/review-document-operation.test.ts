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
