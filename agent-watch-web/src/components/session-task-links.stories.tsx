import '#components/conversation-pane-view.css'

import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionTaskLinks } from '#components/session-task-links'

const openTask = {
  id: 'example-open-task',
  number: 2048,
  title: 'Improve request reliability',
  priority: 1,
  isClosed: false,
}
const linkedTasks = [
  openTask,
  {
    id: 'example-secondary-task',
    number: 2046,
    title: 'Add request tracing',
    priority: 4,
    isClosed: false,
  },
  {
    id: 'example-completed-task',
    number: 2047,
    title: 'Document retry behavior',
    priority: null,
    isClosed: true,
  },
]

const meta = {
  component: SessionTaskLinks,
  args: { task: openTask, tasks: [openTask] },
  decorators: [
    (Story) => (
      <div style={{ width: '760px', minHeight: '140px' }}>
        <div className="conversation-context-line">
          <Story />
        </div>
      </div>
    ),
  ],
} satisfies Meta<typeof SessionTaskLinks>

export default meta
type Story = StoryObj<typeof meta>

export const SessionHasOneLinkedTask: Story = {
  name: 'A session shows its only linked task',
}

export const SessionHasMultipleLinkedTasksAndMenuIsClosed: Story = {
  args: { tasks: linkedTasks, taskListOpen: false },
  name: 'The task menu stays closed while several tasks are linked',
}

export const SessionHasMultipleLinkedTasks: Story = {
  args: {
    tasks: linkedTasks,
    taskListOpen: true,
  },
  name: 'A session lists linked tasks and marks completed work',
  parameters: {
    overflowCheck: {
      ignoreSelectors: ['.session-task-line', '.session-task-links'],
    },
  },
}
