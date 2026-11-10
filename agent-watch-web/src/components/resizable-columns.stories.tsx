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

const meta = {
  component: ResizableColumns,
  args: {
    first: <div style={{ padding: '16px' }}>Session list</div>,
    second: <div style={{ padding: '16px' }}>Conversation</div>,
    storageKey: null,
    firstSize: 360,
    firstMinSize: 240,
    secondMinSize: 320,
    label: 'Resize session list and conversation',
  },
  decorators: [
    (Story) => (
      <div style={{ width: '900px', height: '560px' }}>
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
  name: 'The navigation and main content have an adjustable boundary',
  args: {
    first: (
      <AgentWatchNav
        activeView="inbox"
        paletteShortcut={{
          modifier: 'metaKey',
          label: '⌘K',
          ariaLabel: 'Command K',
        }}
        needsActionCount={1}
        savedViews={[]}
        selectedSavedViewId={null}
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
    ),
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
