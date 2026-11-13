import type { Meta, StoryObj } from '@storybook/react-vite'

import { MobileNavigationActions } from '#components/mobile-navigation-actions'

const meta = {
  component: MobileNavigationActions,
  args: {
    onOpenNavigation: () => undefined,
    onOpenPalette: () => undefined,
  },
  render: (args) => (
    <div className="story-mobile-layout">
      <div className="mobile-inbox-brand">
        <span className="brand-mark" aria-hidden="true">
          &gt;
        </span>
        <span>agent watch</span>
        <MobileNavigationActions {...args} />
        <span className="mobile-inbox-connection">Connected · 10:45</span>
      </div>
    </div>
  ),
} satisfies Meta<typeof MobileNavigationActions>

export default meta
type Story = StoryObj<typeof meta>

export const HeaderControls: Story = {
  name: 'Mobile navigation and search controls sit beside the brand',
}
