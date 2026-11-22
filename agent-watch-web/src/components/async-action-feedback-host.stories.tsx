import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

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
