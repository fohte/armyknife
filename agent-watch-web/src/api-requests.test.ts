import { expect, it } from 'vitest'

import { errorFromResponse } from '#api-requests'

it('accepts close checks with changes and commits', async () => {
  const result = await errorFromResponse(
    new Response(
      JSON.stringify({
        error: 'Close blocked',
        code: 'close_blocked',
        checks: {
          path: '/fictional/worktree',
          files: ['src/fictional.rs'],
          changes: [{ status: 'M', path: 'src/fictional.rs' }],
          commits: [{ sha: 'abc1234', subject: 'preserve fictional work' }],
          branch: 'feature/fictional',
          merged: false,
          reason: 'pull request is open',
          processes: [],
          processInspectionUnavailable: false,
        },
      }),
      { status: 409 },
    ),
  ).match(
    (error) => ({
      message: error.message,
      code: error.code,
      checks: error.checks ?? null,
    }),
    (error) => ({ message: error.message, code: null, checks: null }),
  )

  expect(result).toEqual({
    message: 'Close blocked',
    code: 'close_blocked',
    checks: {
      path: '/fictional/worktree',
      files: ['src/fictional.rs'],
      changes: [{ status: 'M', path: 'src/fictional.rs' }],
      commits: [{ sha: 'abc1234', subject: 'preserve fictional work' }],
      branch: 'feature/fictional',
      merged: false,
      reason: 'pull request is open',
      processes: [],
      processInspectionUnavailable: false,
    },
  })
})

it.each([
  {
    kind: 'change',
    changes: [{ status: 'M' }],
    commits: [],
  },
  {
    kind: 'commit',
    changes: [],
    commits: [{ sha: 'abc1234' }],
  },
])(
  'rejects close checks with an incomplete $kind',
  async ({ changes, commits }) => {
    const result = await errorFromResponse(
      new Response(
        JSON.stringify({
          error: 'Close blocked',
          code: 'close_blocked',
          checks: {
            path: '/fictional/worktree',
            files: [],
            changes,
            commits,
            branch: 'feature/fictional',
            merged: false,
            reason: 'pull request is open',
            processes: [],
            processInspectionUnavailable: false,
          },
        }),
        { status: 409 },
      ),
    ).match(
      (error) => ({ message: error.message, checks: error.checks ?? null }),
      (error) => ({ message: error.message, checks: null }),
    )

    expect(result).toEqual({ message: 'Close blocked', checks: null })
  },
)
