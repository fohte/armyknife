import type { SessionModelOption, SessionModelOptions } from '#new-session-api'

export type NewSessionModelSelection = {
  model: string | null
  effort: string | null
}

export type ReconciledModelSelection = NewSessionModelSelection & {
  modelChanged: boolean
  effortChanged: boolean
}

export function defaultEffortModelOption(
  options: SessionModelOptions,
): SessionModelOption | undefined {
  const configuredModel = options.models.find(
    (option) => option.id === options.defaultModel,
  )
  if (configuredModel !== undefined) return configuredModel

  const [firstModel, ...otherModels] = options.models
  if (firstModel === undefined) return undefined

  return {
    id: options.defaultModel ?? firstModel.id,
    label: 'Default',
    defaultEffort: options.defaultEffort ?? firstModel.defaultEffort,
    efforts: firstModel.efforts.filter((effort) =>
      otherModels.every((model) => model.efforts.includes(effort)),
    ),
  }
}

export function reconcileModelSelection(
  selection: NewSessionModelSelection,
  options: SessionModelOptions,
): ReconciledModelSelection {
  const model =
    selection.model !== null &&
    options.models.some((option) => option.id === selection.model)
      ? selection.model
      : null
  const target =
    model === null
      ? defaultEffortModelOption(options)
      : options.models.find((option) => option.id === model)
  const effort =
    selection.effort === null ||
    (target !== undefined && target.efforts.includes(selection.effort))
      ? selection.effort
      : null

  return {
    model,
    effort,
    modelChanged: model !== selection.model,
    effortChanged: effort !== selection.effort,
  }
}
