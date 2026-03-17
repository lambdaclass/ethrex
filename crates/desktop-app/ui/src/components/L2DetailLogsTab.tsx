import { useLang } from '../App'
import { t } from '../i18n'
import type { L2Config } from './MyL2View'

interface Props {
  l2: L2Config
}

export default function L2DetailLogsTab({ l2 }: Props) {
  const { lang } = useLang()

  return (
    <div className="bg-black rounded-xl p-4 font-mono text-[11px] text-green-400 h-full min-h-[400px] overflow-auto border border-[var(--color-border)]">
      <div className="text-[var(--color-text-secondary)]">[{l2.name}] {t('myl2.detail.logsPlaceholder', lang)}</div>
      <div className="mt-2 text-gray-500">$ ethrex --chain-id {l2.chainId} --port {l2.rpcPort}</div>
      <div className="text-gray-500">INFO: Starting sequencer...</div>
      <div className="text-gray-500">INFO: Listening on 0.0.0.0:{l2.rpcPort}</div>
      <div className="text-gray-500">INFO: Block #1 produced</div>
      <div className="text-gray-500">INFO: Block #2 produced</div>
      <div className="animate-pulse mt-1">▊</div>
    </div>
  )
}
