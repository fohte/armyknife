import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionTaskLinks } from '#components/session-task-links'

const openTask = {
  id: 'example-open-task',
  number: 2048,
  title: 'Improve request reliability',
  priority: 1,
  isClosed: false,
}

const meta = {
  component: SessionTaskLinks,
  args: { task: openTask, tasks: [openTask] },
  decorators: [
    (Story) => (
      <div style={{ width: '420px', minHeight: '140px' }}>
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof SessionTaskLinks>

export default meta
type Story = StoryObj<typeof meta>

export const SessionHasOneLinkedTask: Story = {
  name: 'A session shows its only linked task',
}

export const SessionHasMultipleLinkedTasks: Story = {
  args: {
    tasks: [
      openTask,
      {
        id: 'example-completed-task',
        number: 2047,
        title: 'Document retry behavior',
        priority: null,
        isClosed: true,
      },
    ],
    taskListOpen: true,
  },
  name: 'A session lists linked tasks and marks completed work',
}
