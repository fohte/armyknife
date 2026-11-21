export function containsFiles(dataTransfer: DataTransfer) {
  return Array.from(dataTransfer.types).includes('Files')
}

export function getDroppedFiles(dataTransfer: DataTransfer) {
  return Array.from(dataTransfer.files)
}
