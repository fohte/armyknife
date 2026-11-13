import type { Meta, StoryObj } from '@storybook/react-vite'

import { AgentWatchNav } from '#components/agent-watch-nav'
import { MobileNavDrawer } from '#components/mobile-nav-drawer'

function createNavigation(projectCount: number) {
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
          label: 'Guides',
          projects: Array.from({ length: projectCount }, (_, index) => ({
            name: `Sample project ${String(index + 1)}`,
            status: 'Review' as const,
            needsActionCount: 2,
            sessionId: `sample-session-${String(index + 1)}`,
          })),
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
