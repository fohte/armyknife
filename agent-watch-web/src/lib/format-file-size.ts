export function formatFileSize(bytes: number) {
  if (bytes < 1024) return `${String(bytes)} B`
  const kibibytes = bytes / 1024
  const formatted = new Intl.NumberFormat(undefined, {
    maximumFractionDigits: 1,
  })
  if (kibibytes < 1024) return `${formatted.format(kibibytes)} KiB`
  return `${formatted.format(kibibytes / 1024)} MiB`
}
