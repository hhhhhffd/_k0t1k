import { openUrl } from '@tauri-apps/plugin-opener'
import { isTauri } from '../api/commands'

/** Open a web page in the system browser. */
export async function openExternal(url: string) {
  if (isTauri) await openUrl(url)
  else window.open(url, '_blank', 'noopener')
}
