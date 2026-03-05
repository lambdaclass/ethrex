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
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="px-6 py-4 border-b border-[var(--color-border)]">
        <h1 className="text-lg font-semibold">{t('nodes.title', lang)}</h1>
        <p className="text-xs text-[var(--color-text-secondary)] mt-1">{t('nodes.subtitle', lang)}</p>
      </div>

      <div className="flex-1 overflow-y-auto p-6 space-y-4">
        {nodes.map(node => (
          <div key={node.name} className="bg-[var(--color-bubble-ai)] rounded-xl p-5 flex items-center justify-between">
            <div className="flex items-center gap-4">
              <div className="w-3 h-3 rounded-full" style={{ backgroundColor: statusColor(node.status) }} />
              <div>
                <div className="font-medium">{node.name}</div>
                <div className="text-xs text-[var(--color-text-secondary)] mt-0.5">
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
