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
          commits: [
            {
              sha: '1111111111111111111111111111111111111111',
              subject: 'preserve fictional work',
            },
          ],
          commitInspectionUnavailable: false,
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
      commits: [
        {
          sha: '1111111111111111111111111111111111111111',
          subject: 'preserve fictional work',
        },
      ],
      commitInspectionUnavailable: false,
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
    commitInspectionUnavailable: false,
  },
  {
    kind: 'commit',
    changes: [],
    commits: [{ sha: '1111111111111111111111111111111111111111' }],
    commitInspectionUnavailable: false,
  },
  {
    kind: 'commit inspection status',
    changes: [],
    commits: [],
    commitInspectionUnavailable: 'unknown',
  },
  {
    kind: 'change status',
    changes: [{ status: 'UU', path: 'src/fictional.rs' }],
    commits: [],
    commitInspectionUnavailable: false,
  },
])(
  'rejects close checks with an invalid $kind',
  async ({ changes, commits, commitInspectionUnavailable }) => {
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
            commitInspectionUnavailable,
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
