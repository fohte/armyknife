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
  name: 'The task menu hides completed secondary tasks by default',
}

export const SessionShowsCompletedLinkedTasks: Story = {
  args: {
    tasks: linkedTasks,
    taskListOpen: true,
    completedTasksExpanded: true,
  },
  name: 'The task menu shows completed secondary tasks when expanded',
}

export const SessionHasOnlyCompletedSecondaryTasks: Story = {
  args: {
    task: completedTask,
    tasks: [
      completedTask,
      {
        id: 'example-other-completed-task',
        number: 2045,
        title: 'Review request metrics',
        priority: 3,
        isClosed: true,
      },
      {
        id: 'example-third-completed-task',
        number: 2044,
        title: 'Update retry guide',
        priority: 4,
        isClosed: true,
      },
      {
        id: 'example-fourth-completed-task',
        number: 2043,
        title: 'Verify request alerts',
        priority: 5,
        isClosed: true,
      },
    ],
    taskListOpen: true,
  },
  name: 'A completed primary keeps other completed tasks reachable',
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
