import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'

interface NodeInfo {
  name: string
  status: string | { Error: string }
  pid: number | null
}

const statusColor = (status: NodeInfo['status']) => {
  if (status === 'Running') return 'var(--color-success)'
  if (status === 'Starting') return 'var(--color-warning)'
  if (status === 'Stopped') return 'var(--color-text-secondary)'
  return 'var(--color-error)'
}

const statusText = (status: NodeInfo['status']) => {
  if (typeof status === 'string') return status
  if (typeof status === 'object' && 'Error' in status) return `Error: ${status.Error}`
  return 'Unknown'
}

export default function NodeControlView() {
  const { lang } = useLang()
  const [nodes, setNodes] = useState<NodeInfo[]>([])

  const fetchStatus = async () => {
    try {
      const result = await invoke<NodeInfo[]>('get_all_status')
      setNodes(result)
    } catch (e) {
      console.error(e)
    }
  }

  useEffect(() => { fetchStatus() }, [])

  const handleAction = async (name: string, action: 'start' | 'stop') => {
    try {
      await invoke<string>(`${action}_node`, { name })
      fetchStatus()
    } catch (e) {
      alert(e)
    }
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      <div className="px-4 py-3 border-b border-[var(--color-border)] bg-[var(--color-bg-sidebar)]">
        <h1 className="text-base font-semibold">{t('nodes.title', lang)}</h1>
        <p className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">{t('nodes.subtitle', lang)}</p>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-3">
        {nodes.map(node => (
          <div key={node.name} className="bg-[var(--color-bg-sidebar)] rounded-xl p-4 flex items-center justify-between border border-[var(--color-border)]">
            <div className="flex items-center gap-3">
              <div className="w-3 h-3 rounded-full" style={{ backgroundColor: statusColor(node.status) }} />
              <div>
                <div className="text-[13px] font-medium">{node.name}</div>
                <div className="text-[11px] text-[var(--color-text-secondary)] mt-0.5">
                  {statusText(node.status)} {node.pid ? `(PID: ${node.pid})` : ''}
                </div>
              </div>
            </div>
            <div className="flex gap-2">
              <button
                onClick={() => handleAction(node.name, 'start')}
                disabled={node.status === 'Running'}
                className="bg-[var(--color-success)] text-black text-xs font-medium px-4 py-2 rounded-lg disabled:opacity-30 hover:opacity-80 transition-opacity cursor-pointer"
              >
                {t('nodes.start', lang)}
              </button>
              <button
                onClick={() => handleAction(node.name, 'stop')}
                disabled={node.status === 'Stopped'}
                className="bg-[var(--color-error)] text-white text-xs font-medium px-4 py-2 rounded-lg disabled:opacity-30 hover:opacity-80 transition-opacity cursor-pointer"
              >
                {t('nodes.stop', lang)}
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
