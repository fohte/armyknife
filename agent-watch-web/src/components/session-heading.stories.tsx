import type { Meta, StoryObj } from '@storybook/react-vite'

import { SessionHeading } from '#components/session-heading'

const meta = {
  component: SessionHeading,
  args: {
    title: 'Investigate a failing build',
    isPaused: true,
    task: null,
    tasks: [],
  },
} satisfies Meta<typeof SessionHeading>

export default meta
type Story = StoryObj<typeof meta>

export const PausedSession: Story = {
  name: 'A paused session keeps its title visible',
}

export const ActiveSession: Story = {
  args: { isPaused: false },
  name: 'An active session shows only its title',
}

export const SessionWithLinkedTasks: Story = {
  args: {
    task: {
      id: 'example-open-task',
      number: 2048,
      title: 'Improve request reliability',
      priority: 1,
      isClosed: false,
    },
    tasks: [
      {
        id: 'example-open-task',
        number: 2048,
        title: 'Improve request reliability',
        priority: 1,
        isClosed: false,
      },
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
  name: 'A session shows each linked task and marks completed work',
}
