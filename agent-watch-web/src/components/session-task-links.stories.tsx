import '#components/conversation-pane-view.css'

import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionTaskLinks } from '#components/session-task-links'
import type { SessionTask } from '#types'

const primaryTask: SessionTask = {
  id: 'fixture-primary-task',
  number: 2048,
  title: 'Improve request reliability',
  priority: 1,
}

const secondaryTask: SessionTask = {
  id: 'fixture-secondary-task',
  number: 2046,
  title: 'Add request tracing',
  priority: 4,
}

const meta = {
  component: SessionTaskLinks,
  args: {
    task: primaryTask,
    tasks: [primaryTask],
    onOpenTasks: () => undefined,
  },
  decorators: [
    (Story) => (
      <div className="conversation-pane" style={{ width: 420, height: 100 }}>
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
  name: 'The header shows one primary task without a count',
}

export const SessionHasMultipleLinkedTasks: Story = {
  args: { tasks: [primaryTask, secondaryTask] },
  name: 'The header shows a primary task and a count for another task',
}
