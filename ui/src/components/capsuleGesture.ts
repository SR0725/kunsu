export function shouldOpenCapsuleAfterPointer(dragged: boolean, cancelled: boolean): boolean {
  return !dragged && !cancelled
}
