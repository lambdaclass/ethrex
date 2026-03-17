import { SectionHeader, StatCard, KV } from './ui-atoms'
import type { EconomyMetrics } from './L2DetailView'

interface Props {
  ko: boolean
  econ: EconomyMetrics
}

export default function L2DetailEconomyTab({ ko, econ }: Props) {
  return (
    <>
      {/* TVL & Revenue */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '자산 현황' : 'Assets'} />
        <div className="grid grid-cols-2 gap-2 mt-1">
          <StatCard label="TVL" value={econ.tvl} sub={econ.tvlUsd} />
          <StatCard label={ko ? '수수료 수입' : 'Fee Revenue'} value={econ.gasRevenue} />
        </div>
      </div>

      {/* Gas */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '가스비' : 'Gas Prices'} />
        <div className="grid grid-cols-2 gap-2 mt-1">
          <StatCard label={ko ? 'L1 가스' : 'L1 Gas'} value={`${econ.l1GasPrice} gwei`} />
          <StatCard label={ko ? 'L2 가스' : 'L2 Gas'} value={`${econ.l2GasPrice} gwei`} />
        </div>
      </div>

      {/* Bridge */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '브릿지' : 'Bridge'} />
        <div className="grid grid-cols-2 gap-2 mt-1">
          <StatCard label={ko ? '입금' : 'Deposits'} value={econ.bridgeDeposits.toLocaleString()} />
          <StatCard label={ko ? '출금' : 'Withdrawals'} value={econ.bridgeWithdrawals.toLocaleString()} />
        </div>
      </div>

      {/* Token Info */}
      <div className="bg-[var(--color-bg-sidebar)] rounded-xl p-3 border border-[var(--color-border)]">
        <SectionHeader title={ko ? '토큰 정보' : 'Token Info'} />
        <div className="mt-1 space-y-1.5">
          <KV label={ko ? '네이티브 토큰' : 'Native Token'} value={econ.nativeToken} />
          <div className="flex items-center justify-between text-[11px]">
            <span className="text-[var(--color-text-secondary)]">{ko ? 'L1 토큰 주소' : 'L1 Token'}</span>
            <code className="text-[9px] font-mono text-[#3b82f6] truncate ml-2 max-w-[180px]">{econ.l1TokenAddress}</code>
          </div>
        </div>
      </div>
    </>
  )
}
