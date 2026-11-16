import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionTasksPanel } from '#components/session-tasks-panel'

const meta = {
  component: SessionTasksPanel,
  args: {
    repo: 'example/service',
    tasks: [
      {
        id: 'example-open-task',
        number: 2048,
        title: 'Improve request reliability',
        priority: 1,
      },
      {
        id: 'example-completed-task-a',
        number: 2047,
        title: 'Document request behavior',
        priority: null,
        isClosed: true,
      },
      {
        id: 'example-completed-task-b',
        number: 2046,
        title: 'Add a retry example',
        priority: 3,
        isClosed: true,
      },
    ],
  },
  decorators: [
    (Story) => (
      <div
        className="session-status-panel"
        style={{ height: 540, maxWidth: 540 }}
      >
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionTasksPanel>

export default meta
type Story = StoryObj<typeof meta>

export const InProgressAndCompletedTasks: Story = {
  name: 'The Tasks tab groups open and completed tasks',
}

export const OnlyCompletedTasks: Story = {
  args: {
    tasks: [
      {
        id: 'example-completed-task-a',
        number: 2047,
        title: 'Document request behavior',
        priority: null,
        isClosed: true,
      },
    ],
  },
  name: 'The Tasks tab shows completed tasks when none are open',
}
