// 邊緣瀏海示範（AC-WEB-02）：放開時吸到最近的邊，記住沿邊的位置（0～1）。
export type Edge = 'top' | 'right' | 'bottom' | 'left'

export function snapToEdge(x: number, y: number, w: number, h: number): { edge: Edge; along: number } {
  const cx = Math.min(Math.max(x, 0), w)
  const cy = Math.min(Math.max(y, 0), h)
  const d: [Edge, number][] = [['top', cy], ['right', w - cx], ['bottom', h - cy], ['left', cx]]
  d.sort((a, b) => a[1] - b[1])
  const edge = d[0][0]
  const along = edge === 'top' || edge === 'bottom' ? cx / w : cy / h
  return { edge, along: Math.min(Math.max(along, 0), 1) }
}
