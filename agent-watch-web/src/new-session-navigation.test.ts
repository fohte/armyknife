import { expect, it } from 'vitest'

import { findLaunchedSession } from '#new-session-navigation'
import type { Session } from '#types'

const session: Session = {
  id: 'session-fixture',
  launchId: 'launch-fixture',
  category: 'in_progress',
  isPaused: false,
  kind: null,
  title: 'Prepare the service configuration',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'example/service',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

it('finds the feed session carrying the returned launch ID', () => {
  expect(
    findLaunchedSession(
      [{ ...session, id: 'other-session', launchId: 'other-launch' }, session],
      'launch-fixture',
    ),
  ).toEqual(session)
})
