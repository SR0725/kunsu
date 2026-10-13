// 鍵盤操作的純函數（可測）。

/** 分段控制的方向鍵：左右上下循環、Home／End 到兩端；其他鍵回 null（交給瀏覽器）。 */
export function segmentKey(index: number, count: number, key: string): number | null {
  if (count <= 0) return null
  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
      return (index + 1) % count
    case 'ArrowLeft':
    case 'ArrowUp':
      return (index - 1 + count) % count
    case 'Home':
      return 0
    case 'End':
      return count - 1
    default:
      return null
  }
}
