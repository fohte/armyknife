import '#components/conversation-pane-view.css'

import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionTaskLinks } from '#components/session-task-links'
import type { Session, SessionTask } from '#types'

const openTask = {
  id: 'example-open-task',
  number: 2048,
  title: 'Improve request reliability',
  priority: 1,
  isClosed: false,
}
const secondaryTask = {
  id: 'example-secondary-task',
  number: 2046,
  title: 'Add request tracing',
  priority: 4,
  isClosed: false,
}
const completedTask = {
  id: 'example-completed-task',
  number: 2047,
  title: 'Document retry behavior',
  priority: null,
  isClosed: true,
}
const linkedTasks = [openTask, secondaryTask, completedTask]
function session(category: Session['category'], tasks: SessionTask[]) {
  return { category, task: tasks[0] ?? null, tasks }
}
const linkedSessions = [
  session('needs_action', [openTask]),
  session('needs_action', [openTask]),
  session('in_progress', [openTask, secondaryTask]),
  session('done', [openTask, secondaryTask]),
  session('later', [completedTask]),
]

const meta = {
  component: SessionTaskLinks,
  args: { task: openTask, tasks: [openTask], sessions: linkedSessions },
  decorators: [
    (Story, context) => (
      <div
        className="conversation-pane"
        style={{
          width:
            typeof context.parameters['conversationPaneWidth'] === 'number'
              ? context.parameters['conversationPaneWidth']
              : 760,
          height: '140px',
        }}
      >
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
}

export const SessionHasMultipleLinkedTasksInNarrowPane: Story = {
  args: {
    tasks: linkedTasks,
    taskListOpen: true,
  },
  name: 'A narrow conversation pane keeps the task menu visible',
  parameters: {
    conversationPaneWidth: 240,
  },
}
