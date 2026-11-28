import { expect, it } from 'vitest'

import {
  defaultEffortModelOption,
  reconcileModelSelection,
} from '#new-session-selection'

it('offers only shared effort levels when the default model is unknown', () => {
  expect(
    defaultEffortModelOption({
      defaultModel: 'fixture-profile-model',
      defaultEffort: 'medium',
      models: [
        {
          id: 'fixture-model-orbit',
          label: 'Orbit',
          defaultEffort: 'low',
          efforts: ['low', 'medium', 'high'],
        },
        {
          id: 'fixture-model-comet',
          label: 'Comet',
          defaultEffort: 'medium',
          efforts: ['medium', 'high'],
        },
      ],
    }),
  ).toEqual({
    id: 'fixture-profile-model',
    label: 'Default',
    defaultEffort: 'medium',
    efforts: ['medium', 'high'],
  })
})

it('uses the profile effort when the configured model is in the catalog', () => {
  expect(
    defaultEffortModelOption({
      defaultModel: 'fixture-model-orbit',
      defaultEffort: 'high',
      models: [
        {
          id: 'fixture-model-orbit',
          label: 'Orbit',
          defaultEffort: 'medium',
          efforts: ['low', 'medium', 'high'],
        },
      ],
    }),
  ).toEqual({
    id: 'fixture-model-orbit',
    label: 'Orbit',
    defaultEffort: 'high',
    efforts: ['low', 'medium', 'high'],
  })
})

it('clears an unavailable model and keeps a supported effort', () => {
  expect(
    reconcileModelSelection(
      { model: 'fixture-model-orbit', effort: 'xhigh' },
      {
        defaultModel: 'fixture-model-comet',
        defaultEffort: 'medium',
        models: [
          {
            id: 'fixture-model-comet',
            label: 'Comet',
            defaultEffort: 'medium',
            efforts: ['medium', 'xhigh'],
          },
        ],
      },
    ),
  ).toEqual({
    model: null,
    effort: 'xhigh',
  })
})

it('clears an effort unavailable to the selected model', () => {
  expect(
    reconcileModelSelection(
      { model: 'fixture-model-orbit', effort: 'xhigh' },
      {
        defaultModel: 'fixture-model-comet',
        defaultEffort: 'medium',
        models: [
          {
            id: 'fixture-model-orbit',
            label: 'Orbit',
            defaultEffort: 'low',
            efforts: ['low', 'medium'],
          },
          {
            id: 'fixture-model-comet',
            label: 'Comet',
            defaultEffort: 'medium',
            efforts: ['medium'],
          },
        ],
      },
    ),
  ).toEqual({
    model: 'fixture-model-orbit',
    effort: null,
  })
})
