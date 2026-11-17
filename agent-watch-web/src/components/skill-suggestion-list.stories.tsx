import type { Meta, StoryObj } from '@storybook/react-vite'

import { SkillSuggestionList } from '#components/skill-completion'

const meta = {
  component: SkillSuggestionList,
  args: {
    id: 'skill-suggestions',
    completion: { trigger: '/', query: '', start: 0, end: 1 },
    skills: [
      { name: 'design-flow', description: 'Help shape a design.' },
      { name: 'debug-flow', description: 'Investigate and fix errors.' },
    ],
    selectedIndex: 0,
    onSelect: () => undefined,
  },
} satisfies Meta<typeof SkillSuggestionList>

export default meta
type Story = StoryObj<typeof meta>

export const ClaudeSkills: Story = {
  name: 'Claude skills appear under the slash trigger',
}

export const CodexSkillSelected: Story = {
  name: 'Codex highlights the selected skill under the dollar trigger',
  args: {
    completion: { trigger: '$', query: '', start: 0, end: 1 },
    selectedIndex: 1,
  },
}
