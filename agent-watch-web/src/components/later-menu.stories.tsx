import type { Meta, StoryObj } from '@storybook/react-vite'

import { LaterMenu } from '#components/later-menu'

const meta = {
  component: LaterMenu,
  args: {
    sessionTitle: 'Session awaiting a decision',
    onSelect: () => {},
    onClose: () => {},
  },
} satisfies Meta<typeof LaterMenu>

export default meta
type Story = StoryObj<typeof meta>

export const MenuOffersFiveReturnTimes: Story = {
  name: 'the menu offers five return times',
}
