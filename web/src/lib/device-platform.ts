/** Platforms supported by native device registration, independent of browsers. */
export const devicePlatforms = ['macos', 'windows', 'linux', 'android'] as const
export type DevicePlatform = (typeof devicePlatforms)[number]
export function isDevicePlatform(value: unknown): value is DevicePlatform {
  return (
    typeof value === 'string' &&
    devicePlatforms.some((platform) => platform === value)
  )
}
