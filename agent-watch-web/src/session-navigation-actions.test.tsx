import { expect, it } from 'vitest'

import type { SessionHistoryState } from '#session-history'
import { createSessionNavigationActions } from '#session-navigation-actions'
import type { Session } from '#types'

function session(id: string, category: Session['category']): Session {
  return {
    id,
    category,
    isPaused: false,
    kind: 'Question',
    title: 'Review a request',
    preview: 'Please choose a retry policy.',
    elapsedSeconds: 10,
    unread: true,
    task: null,
    repo: 'example/service',
    project: 'sample project',
    ancestorSessionIds: [],
    parent: null,
    pullRequest: null,
  }
}

it('requests composer focus through shared session-opening actions', () => {
  const focusRequests: string[] = []
  const navigations: Array<{
    state: SessionHistoryState
    mode: 'push' | 'replace' | undefined
  }> = []
  const expandedStates: boolean[] = []
  const initialState: SessionHistoryState = {
    selectedId: null,
    view: 'inbox',
    conversationOpen: false,
    newSessionMode: null,
  }
  const {
    selectSession,
    openConversation,
    openSessionInInbox,
    openTodaySession,
  } = createSessionNavigationActions({
    sessions: [session('sample-later-session', 'later')],
    requestComposerFocus: (sessionId) => focusRequests.push(sessionId),
    navigate: (update, mode) => {
      navigations.push({ state: update(initialState), mode })
    },
    setLaterExpanded: (expanded) => expandedStates.push(expanded),
    toggleToday: () => undefined,
  })

  openConversation('sample-conversation-session')
  selectSession('sample-linked-session')
  openSessionInInbox('sample-inbox-session', 'replace')
  openTodaySession('sample-later-session')

  const snapshot = () => ({ focusRequests, navigations, expandedStates })
  expect(snapshot()).toEqual({
    focusRequests: [
      'sample-conversation-session',
      'sample-linked-session',
      'sample-inbox-session',
      'sample-later-session',
    ],
    navigations: [
      {
        state: {
          selectedId: 'sample-conversation-session',
          view: 'inbox',
          conversationOpen: true,
          newSessionMode: null,
        },
        mode: undefined,
      },
      {
        state: {
          selectedId: 'sample-linked-session',
          view: 'inbox',
          conversationOpen: false,
          newSessionMode: null,
        },
        mode: undefined,
      },
      {
        state: {
          selectedId: 'sample-inbox-session',
          view: 'inbox',
          conversationOpen: true,
          newSessionMode: null,
        },
        mode: 'replace',
      },
      {
        state: {
          selectedId: 'sample-later-session',
          view: 'inbox',
          conversationOpen: true,
          newSessionMode: null,
        },
        mode: undefined,
      },
    ],
    expandedStates: [true],
  })
})
