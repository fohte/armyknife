import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ComponentType } from 'react'

import { BackgroundTaskRowView } from '#components/running-task-row-view'
import { RunningTasksPanelView } from '#components/running-tasks-panel-view'
import type { BackgroundTask } from '#types'

const taskStates = {
  subagents: [
    {
      id: 'fixture-agent-task',
      description: '再送処理の呼び出し元を探す',
      elapsedSeconds: 42,
      kind: 'subagent',
      agentType: 'Explore',
    },
  ] satisfies BackgroundTask[],
  backgroundTaskRows: [
    <BackgroundTaskRowView
      key="fixture-shell-task"
      task={{
        id: 'fixture-shell-task',
        description: 'cargo test -p service',
        elapsedSeconds: 65,
        kind: 'background_task',
      }}
      isOpen
      output={[
        'running 24 tests',
        'test retry::stops_after_limit ... ok',
        'test retry::backs_off ... ok',
      ].join('\n')}
      error={null}
      onToggle={() => undefined}
    />,
  ],
}

const desktopViewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="desktop"
    style={{ width: '640px', height: '900px', display: 'flex' }}
  >
    <Story />
  </div>
)

const phoneViewport = (Story: ComponentType) => (
  <div
    className="session-status-panel"
    data-size="phone"
    style={{ width: '390px', height: '844px', display: 'flex' }}
  >
    <Story />
  </div>
)

const meta = {
  component: RunningTasksPanelView,
  args: taskStates,
  decorators: [desktopViewport],
} satisfies Meta<typeof RunningTasksPanelView>

export default meta
type Story = StoryObj<typeof meta>

export const DesktopOutputIsExpanded: Story = {
  name: 'The desktop status panel shows the expanded task output.',
}

export const PhoneOutputIsExpanded: Story = {
  name: 'The phone status page shows the expanded task output.',
  decorators: [phoneViewport],
}
