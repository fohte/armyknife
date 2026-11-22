import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import {
  type AsyncAction,
  isAsyncActionFailureStatus,
} from '#async-action-feedback'
import { AgentWatchNav } from '#components/agent-watch-nav'
import { AsyncActionFeedbackHost } from '#components/async-action-feedback-host'
import type { Session } from '#types'

const phoneSheetStarredSession: Session = {
  id: 'phone-sheet-starred-session',
  engine: 'claude',
  workType: 'development',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Review the example rollout',
  preview: 'Choose a limit.',
  elapsedSeconds: 10,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'example project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  isToday: true,
}

const phoneSheetPmSession: Session = {
  ...phoneSheetStarredSession,
  id: 'phone-sheet-pm-session',
  workType: 'PM',
  category: 'in_progress',
  kind: null,
  title: 'Review the example monitor',
  unread: false,
}

const desktopNavStarredSession: Session = {
  id: 'desktop-nav-starred-session',
  engine: 'claude',
  workType: 'development',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Review the notification route',
  preview: 'Choose a retry limit.',
  elapsedSeconds: 10,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'example project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  isToday: true,
}

function withDesktopLayout(
  Story: ComponentType,
  context: { args: { actions: readonly AsyncAction[] } },
) {
  const actions = context.args.actions
  const failed = actions.some((action) =>
    isAsyncActionFailureStatus(action.status),
  )
  const failedCount = actions.filter((action) =>
    isAsyncActionFailureStatus(action.status),
  ).length
  const runningCount = actions.filter(
    (action) => action.status === 'merging' || action.status === 'closing',
  ).length
  const count = failed
    ? failedCount
    : runningCount > 0
      ? runningCount
      : actions.length
  return (
    <div
      className="desktop-activity-story"
      style={{
        position: 'relative',
        display: 'flex',
        width: '100vw',
        height: '100vh',
        overflow: 'hidden',
        background: 'var(--aw-bg-app)',
      }}
    >
      <div
        style={{
          display: 'flex',
          width: 240,
          flex: 'none',
          background: 'var(--aw-bg-nav)',
        }}
      >
        <AgentWatchNav
          activeView="inbox"
          paletteShortcut={{
            modifier: 'metaKey',
            label: '⌘K',
            ariaLabel: 'Command K',
          }}
          needsActionCount={5}
          savedViews={[]}
          selectedSavedViewId={null}
          workTypeNavGroups={[]}
          onViewChange={() => undefined}
          onSelectSavedView={() => undefined}
          onDeleteSavedView={() => undefined}
          todaySessions={[desktopNavStarredSession]}
          onSelectToday={() => undefined}
          onSelectSession={() => undefined}
          onOpenPalette={() => undefined}
          activityCount={count}
          activityFailed={failed}
        />
      </div>
      <div
        style={{
          width: 460,
          flex: 'none',
          borderRight: '1px solid var(--aw-border-control)',
          background: 'var(--aw-bg-surface)',
        }}
      >
        <header
          style={{
            height: 48,
            borderBottom: '1px solid var(--aw-border-control)',
          }}
        />
      </div>
      <main style={{ minWidth: 0, flex: 1 }}>
        <header
          style={{
            height: 48,
            borderBottom: '1px solid var(--aw-border-control)',
          }}
        />
      </main>
      <Story />
    </div>
  )
}

const desktopFailureAction = {
  id: 1,
  kind: 'close' as const,
  sessionId: 'example-close-session',
  sessionTitle: 'Review the notification route',
  status: 'closeFailed' as const,
  error: 'The example workspace could not be closed.',
  checks: {
    path: '~/work/example-service',
    files: [],
    branch: 'feature/example-route',
    merged: true,
    reason: null,
    processes: [
      { pid: 23451, name: 'example server' },
      { pid: 23452, name: 'example worker' },
    ],
    processInspectionUnavailable: false,
  },
  createdAt: 1_799_000_000_000,
}

const desktopActivityAction = {
  id: 2,
  kind: 'merge' as const,
  sessionId: 'example-merge-session',
  sessionTitle: 'Review the notification route',
  pullRequestNumber: 48,
  status: 'mergeFailed' as const,
  error:
    'The example branch changed after the latest review. Confirm the new version before merging.',
  createdAt: 1_799_000_000_003,
}

const desktopClosedAction = {
  id: 3,
  kind: 'close' as const,
  sessionId: 'example-closed-session',
  sessionTitle: 'Review the deployment monitor',
  status: 'closed' as const,
  createdAt: 1_799_000_000_002,
}

const desktopQueuedAction = {
  id: 4,
  kind: 'merge' as const,
  sessionId: 'example-queued-session',
  sessionTitle: 'Review the rollout monitor',
  pullRequestNumber: 49,
  status: 'queued' as const,
  message: 'The example merge is waiting for checks.',
  createdAt: 1_798_999_700_000,
}

const desktopAlreadyMergedAction = {
  id: 5,
  kind: 'merge' as const,
  sessionId: 'example-already-merged-session',
  sessionTitle: 'Review the retry limit',
  pullRequestNumber: 50,
  status: 'alreadyMerged' as const,
  createdAt: 1_798_999_000_000,
}

const desktopMergedAction = {
  id: 6,
  kind: 'merge' as const,
  sessionId: 'example-merged-session',
  sessionTitle: 'Review the alert wording',
  pullRequestNumber: 51,
  status: 'merged' as const,
  createdAt: 1_798_998_500_000,
}

const desktopClosedEarlierAction = {
  ...desktopClosedAction,
  id: 7,
  sessionTitle: 'Review the monitoring schedule',
  createdAt: 1_798_999_300_000,
}

const desktopMergedEarlierAction = {
  ...desktopMergedAction,
  id: 8,
  sessionId: 'example-prior-merge-session',
  sessionTitle: 'Review the fallback route',
  pullRequestNumber: 52,
  createdAt: 1_798_998_900_000,
}

function withPhoneNavigation(activityCount: number, activityFailed: boolean) {
  return (Story: ComponentType) => (
    <div
      className="story-mobile-layout"
      style={{
        position: 'relative',
        boxSizing: 'border-box',
        width: '100%',
        maxWidth: 390,
        height: 844,
        overflow: 'hidden',
        background: 'var(--aw-bg-app)',
      }}
    >
      <div
        aria-hidden="true"
        style={{
          position: 'absolute',
          inset: '0 auto 0 0',
          display: 'flex',
          width: 320,
          height: '100%',
          background: 'var(--aw-bg-nav)',
        }}
      >
        <AgentWatchNav
          activeView="inbox"
          paletteShortcut={{
            modifier: 'metaKey',
            label: '⌘K',
            ariaLabel: 'Command K',
          }}
          needsActionCount={3}
          savedViews={[]}
          selectedSavedViewId={null}
          workTypeNavGroups={[{ label: 'PM', sessions: [phoneSheetPmSession] }]}
          onViewChange={() => undefined}
          onSelectSavedView={() => undefined}
          onDeleteSavedView={() => undefined}
          todaySessions={[phoneSheetStarredSession]}
          onSelectToday={() => undefined}
          onSelectSession={() => undefined}
          onOpenPalette={() => undefined}
          isMobile
          activityCount={activityCount}
          activityFailed={activityFailed}
        />
      </div>
      <Story />
    </div>
  )
}

const meta = {
  component: AsyncActionFeedbackHost,
  args: {
    actions: [
      {
        id: 1,
        kind: 'merge',
        sessionId: 'sample-session',
        sessionTitle: 'Review the deployment plan',
        pullRequestNumber: 42,
        status: 'mergeFailed',
        error: 'The branch changed after confirmation.',
        createdAt: 1_800_000_000_000,
      },
    ],
    toastIds: [1],
    activityOpen: false,
    activityPosition: null,
    toastBottomOffset: null,
    selectedActionId: null,
    retry: () => undefined,
    dismissToast: () => undefined,
    clearFinished: () => undefined,
    openActivity: () => undefined,
    closeActivity: () => undefined,
    closeWithWorktree: () => undefined,
    openInTerminal: () => undefined,
  },
} satisfies Meta<typeof AsyncActionFeedbackHost>

export default meta
type Story = StoryObj<typeof meta>

export const FailureToastIsVisible: Story = {
  name: 'A failed merge stays visible as a toast.',
}

export const ActivityKeepsTheResult: Story = {
  name: 'Activity lists results and can retry failures.',
  args: { activityOpen: true, toastIds: [] },
}

export const ActivityShowsEmptyState: Story = {
  name: 'Activity explains where merge and close results will appear.',
  args: { actions: [], toastIds: [], activityOpen: true },
}

export const DesktopActivityShowsEmptyState: Story = {
  name: 'The desktop Activity opens beside its navigation row when empty.',
  parameters: { layout: 'fullscreen' },
  args: {
    actions: [],
    toastIds: [],
    activityOpen: true,
    activityPosition: { left: 248, bottom: 148 },
  },
  decorators: [withDesktopLayout],
}

export const DesktopActivityShowsResults: Story = {
  name: 'The desktop Activity lists recent action results.',
  parameters: { layout: 'fullscreen' },
  args: {
    actions: [
      desktopActivityAction,
      desktopClosedAction,
      desktopQueuedAction,
      desktopAlreadyMergedAction,
      desktopMergedAction,
    ],
    toastIds: [],
    activityOpen: true,
    activityPosition: { left: 248, bottom: 148 },
  },
  decorators: [withDesktopLayout],
}

export const DesktopActivityShowsCloseFailure: Story = {
  name: 'The desktop Activity shows why a failed close needs attention.',
  parameters: { layout: 'fullscreen' },
  args: {
    actions: [
      desktopFailureAction,
      desktopClosedEarlierAction,
      desktopMergedEarlierAction,
    ],
    toastIds: [],
    activityOpen: true,
    activityPosition: { left: 248, bottom: 148 },
  },
  decorators: [withDesktopLayout],
}

export const PhoneShowsEmptyActivity: Story = {
  name: 'The phone sheet explains where merge and close results appear.',
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'mobile2' },
  },
  args: { actions: [], toastIds: [], activityOpen: true },
  decorators: [withPhoneNavigation(0, false)],
}

export const PhoneShowsRecentActivity: Story = {
  name: 'The phone sheet lists running, failed, and successful actions.',
  parameters: {
    layout: 'fullscreen',
    viewport: { defaultViewport: 'mobile2' },
  },
  args: {
    actions: [
      {
        id: 1,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example deployment',
        pullRequestNumber: 42,
        status: 'mergeFailed',
        error: 'The example branch changed after confirmation.',
        createdAt: 1_800_000_000_003,
      },
      {
        id: 2,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example monitor',
        pullRequestNumber: 43,
        status: 'merged',
        createdAt: 1_800_000_000_002,
      },
      {
        id: 3,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example rollout',
        pullRequestNumber: 44,
        status: 'merging',
        createdAt: 1_800_000_000_001,
      },
    ],
    toastIds: [],
    activityOpen: true,
  },
  decorators: [withPhoneNavigation(1, true)],
}

export const ToastsSitAboveAnOpenPhoneSheet: Story = {
  name: 'The phone action toast sits above an open sheet.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    toastBottomOffset: 360,
    actions: [
      {
        id: 1,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example deployment',
        pullRequestNumber: 42,
        status: 'queued',
        message: 'It merges when the queue checks pass.',
        createdAt: 1_800_000_000_000,
      },
    ],
  },
  decorators: [
    (Story) => (
      <div
        className="story-mobile-layout"
        style={{
          boxSizing: 'border-box',
          width: '100%',
          maxWidth: 390,
          height: 844,
        }}
      >
        <div
          aria-label="Open pull request sheet"
          style={{
            position: 'absolute',
            right: 0,
            bottom: 0,
            left: 0,
            height: 320,
            background: 'var(--aw-bg-nav)',
            borderTop: '1px solid var(--aw-border-control)',
          }}
        />
        <Story />
      </div>
    ),
  ],
}

export const PhoneShowsClosedSessionAndMergeFailure: Story = {
  name: 'The phone keeps a closed session and failed merge visible together.',
  parameters: { viewport: { defaultViewport: 'mobile2' } },
  args: {
    toastBottomOffset: null,
    actions: [
      {
        id: 1,
        kind: 'close',
        sessionId: 'example-session',
        sessionTitle: 'Review the example configuration',
        status: 'closed',
        createdAt: 1_800_000_000_000,
      },
      {
        id: 2,
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Review the example deployment',
        pullRequestNumber: 43,
        status: 'mergeFailed',
        error: 'The example branch changed after confirmation.',
        createdAt: 1_800_000_000_001,
      },
    ],
    toastIds: [2, 1],
  },
}
