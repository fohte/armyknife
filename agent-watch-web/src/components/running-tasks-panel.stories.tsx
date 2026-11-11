import type { Meta, StoryObj } from '@storybook/react-vite'

import { RunningTasksPanel } from '#components/running-tasks-panel'

const meta = {
  component: RunningTasksPanel,
  args: {
    tasks: [
      {
        id: 'fixture-agent-task',
        description: 'Inspect the fictional module structure',
        elapsedSeconds: 72,
        kind: 'subagent',
        agentType: 'FixtureExplorer',
      },
      {
        id: 'fixture-shell-task',
        description: 'cargo test -p fictional-crate',
        elapsedSeconds: 31,
        kind: 'background_task',
      },
    ],
  },
  decorators: [
    (Story) => (
      <div style={{ width: '360px', height: '480px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof RunningTasksPanel>

export default meta
type Story = StoryObj<typeof meta>

export const SubagentsAndBackgroundTasks: Story = {
  name: 'The panel groups running subagents and background tasks',
}

export const OnlyBackgroundTasks: Story = {
  name: 'The panel shows the background tasks section on its own',
  args: {
    tasks: [
      {
        id: 'fixture-shell-task',
        description: 'cargo test -p fictional-crate',
        elapsedSeconds: 31,
        kind: 'background_task',
      },
    ],
  },
}
