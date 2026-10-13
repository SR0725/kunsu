// 草稿資產（AC-EC-08）：內文的 ./assets/x.png 換成該版本快照的 data URI；其他網址原樣交給 sanitizer。
export function resolveAsset(src: string | undefined, assets: Record<string, string>): string | undefined {
  if (src === undefined) return undefined
  const key = src.startsWith('./') ? src : src.startsWith('assets/') ? `./${src}` : src
  return assets[key] ?? src
}
