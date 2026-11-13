import { requestJson } from '#api-requests'
import {
  LATER_PRESET_OPTIONS,
  type LaterPresetTimes,
} from '#session-navigation'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isLaterPresetTimes(value: unknown): value is LaterPresetTimes {
  if (!isRecord(value)) {
    return false
  }
  return LATER_PRESET_OPTIONS.every(({ preset }) => {
    if (!Object.hasOwn(value, preset)) {
      return false
    }
    const option = value[preset]
    return (
      isRecord(option) &&
      Object.hasOwn(option, 'when') &&
      typeof option['when'] === 'string' &&
      Object.hasOwn(option, 'deadline') &&
      (option['deadline'] === null || typeof option['deadline'] === 'string')
    )
  })
}

export function getLaterPresetTimes() {
  return requestJson('/api/sessions/later-options', isLaterPresetTimes)
}
