import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'
import { MobileNavDrawer } from '#components/mobile-nav-drawer'
import type { Session } from '#types'

function createNavigation(sessionCount: number, waiting = false) {
  const sessions: Session[] = Array.from(
    { length: sessionCount },
    (_, index) => ({
      id: `sample-session-${String(index + 1)}`,
      engine: 'codex',
      workType: 'PM',
      category: waiting ? 'waiting' : 'needs_action',
      isPaused: false,
      kind: waiting ? null : 'Question',
      title: `Review sample change ${String(index + 1)}`,
      preview: waiting ? null : 'Choose an example option.',
      waitingReason: waiting ? 'Waiting for the checks to finish' : null,
      elapsedSeconds: 30,
      unread: false,
      task: null,
      repo: 'example/project',
      project: 'sample project',
      parent: null,
      ancestorSessionIds: [],
      pullRequest: null,
    }),
  )

  return (
    <AgentWatchNav
      activeView="inbox"
      isMobile
      paletteShortcut={{
        modifier: 'metaKey',
        label: '⌘K',
        ariaLabel: 'Command K',
      }}
      needsActionCount={2}
      savedViews={[
        {
          id: 'reviews',
          name: 'Sample reviews',
          query: 'project:sample kind:review',
          count: 2,
          countTone: 'needs-action',
        },
      ]}
      selectedSavedViewId={null}
      workTypeNavGroups={[
        {
          label: 'PM',
          sessions,
        },
      ]}
      onViewChange={() => undefined}
      onSelectSavedView={() => undefined}
      onDeleteSavedView={() => undefined}
      todaySessions={[]}
      onSelectToday={() => undefined}
      onSelectSession={() => undefined}
      onOpenPalette={() => undefined}
    />
  )
}

const meta = {
  component: MobileNavDrawer,
  args: {
    open: true,
    onClose: () => undefined,
    children: createNavigation(1),
  },
  render: (args) => (
    <div className="agent-watch-shell" data-layout="mobile">
      <main className="agent-watch-main" />
      <MobileNavDrawer {...args} />
    </div>
  ),
} satisfies Meta<typeof MobileNavDrawer>

export default meta
type Story = StoryObj<typeof meta>

export const Open: Story = {
  name: 'The mobile navigation is open as a left drawer',
}

export const LongNavigation: Story = {
  name: 'The drawer scrolls when navigation is taller than the screen',
  args: {
    children: createNavigation(30),
  },
}

export const WaitingSession: Story = {
  name: 'The drawer lists a waiting session',
  args: { children: createNavigation(1, true) },
}
