import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'
import { ResizableColumns } from '#components/resizable-columns'
import type { Session } from '#types'

const todaySession: Session = {
  id: 'storybook-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Review',
  title: 'Review the example change',
  preview: null,
  elapsedSeconds: 30,
  unread: false,
  task: null,
  repo: 'example/project',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
  isToday: true,
}

function navigation(activeView: 'inbox' | 'board') {
  return (
    <AgentWatchNav
      activeView={activeView}
      paletteShortcut={{
        modifier: 'metaKey',
        label: '⌘K',
        ariaLabel: 'Command K',
      }}
      needsActionCount={3}
      savedViews={[
        {
          id: 'needs-review',
          name: 'Needs review',
          query: 'kind:review',
          count: 2,
          countTone: 'needs-action',
        },
        {
          id: 'errors',
          name: 'Errors',
          query: 'kind:error',
          count: 1,
          countTone: 'error',
        },
      ]}
      selectedSavedViewId="needs-review"
      todaySessions={[todaySession]}
      workTypeNavGroups={[
        {
          label: 'PM',
          projects: [
            {
              name: 'sample project',
              status: 'Review',
              needsActionCount: 1,
              sessionId: todaySession.id,
            },
          ],
        },
      ]}
      onViewChange={() => undefined}
      onSelectSavedView={() => undefined}
      onDeleteSavedView={() => undefined}
      onSelectToday={() => undefined}
      onSelectSession={() => undefined}
      onOpenPalette={() => undefined}
    />
  )
}

const meta = {
  component: ResizableColumns,
  args: {
    first: navigation('inbox'),
    second: <div style={{ padding: '16px' }}>Conversation</div>,
    storageKey: null,
    firstSize: 360,
    firstMinSize: 240,
    secondMinSize: 320,
    label: 'Resize session list and conversation',
  },
  decorators: [
    (Story) => (
      <div style={{ width: '100vw', height: '100vh' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ResizableColumns>

export default meta
type Story = StoryObj<typeof meta>

export const TwoColumnsHaveAnAdjustableBoundary: Story = {
  name: 'The session list and conversation have an adjustable boundary',
}

export const FirstPaneIsCollapsed: Story = {
  name: 'The first pane collapses while the second pane fills the space',
  args: { firstCollapsed: true },
}

export const NavigationHasAnAdjustableBoundary: Story = {
  name: 'The navigation and main content meet at 240px',
  args: {
    first: navigation('inbox'),
    second: (
      <main className="agent-watch-main">
        <div style={{ padding: '16px' }}>Inbox and conversation panes</div>
      </main>
    ),
    firstSize: 240,
    firstMinSize: 160,
    secondMinSize: 508,
    label: 'Resize navigation and content',
  },
}

export const BoardHasTheSameNavigationBoundary: Story = {
  name: 'The Board view keeps the navigation at 240px',
  args: {
    first: navigation('board'),
    second: (
      <main className="agent-watch-main">
        <div style={{ padding: '16px' }}>Board</div>
      </main>
    ),
    firstSize: 240,
    firstMinSize: 160,
    secondMinSize: 508,
    label: 'Resize navigation and Board',
  },
}
