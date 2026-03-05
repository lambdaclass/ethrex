import { useState, useRef, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useLang } from '../App'
import { t } from '../i18n'

interface Message {
  role: 'user' | 'assistant'
  content: string
}

export default function ChatView() {
  const { lang } = useLang()
  const [messages, setMessages] = useState<Message[]>([
    { role: 'assistant', content: t('chat.welcome', lang) }
  ])
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages])

  const sendMessage = async () => {
    if (!input.trim() || loading) return
    const userMsg: Message = { role: 'user', content: input }
    setMessages(prev => [...prev, userMsg])
    setInput('')
    setLoading(true)
    try {
      const response = await invoke<{ role: string; content: string }>('send_chat_message', { message: input })
      setMessages(prev => [...prev, { role: 'assistant', content: response.content }])
    } catch (e) {
      setMessages(prev => [...prev, { role: 'assistant', content: `Error: ${e}` }])
    } finally {
      setLoading(false)
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMessage() }
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-main)]">
      {/* Header */}
      <div className="px-4 py-3 border-b border-[var(--color-border)] flex items-center justify-between bg-[var(--color-bg-sidebar)]">
        <div className="flex items-center gap-2.5">
          <div className="w-9 h-9 rounded-full bg-[var(--color-accent)] flex items-center justify-center text-sm">🤖</div>
          <div>
            <div className="text-sm font-semibold">{t('chat.title', lang)}</div>
            <div className="text-[11px] text-[var(--color-text-secondary)]">{t('chat.notConnected', lang)}</div>
          </div>
        </div>
        <select className="bg-[var(--color-bg-sidebar)] text-xs rounded-lg px-2 py-1 outline-none border border-[var(--color-border)]">
          <option>Claude</option>
          <option>GPT</option>
          <option>Gemini</option>
        </select>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-2.5 bg-[var(--color-bg-chat)]">
        {messages.map((msg, i) => (
          <div key={i} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div
              className={`max-w-[80%] rounded-2xl px-3 py-2 text-[13px] whitespace-pre-wrap leading-relaxed shadow-sm ${
                msg.role === 'user'
                  ? 'bg-[var(--color-bubble-user)] text-[var(--color-accent-text)] rounded-br-sm'
                  : 'bg-[var(--color-bubble-ai)] text-[var(--color-text-primary)] rounded-bl-sm'
              }`}
            >
              {msg.content}
            </div>
          </div>
        ))}
        {loading && (
          <div className="flex justify-start">
            <div className="bg-[var(--color-bubble-ai)] rounded-2xl rounded-bl-sm px-3 py-2 text-[13px] shadow-sm">
              <span className="animate-pulse text-[var(--color-text-secondary)]">{t('chat.thinking', lang)}</span>
            </div>
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input */}
      <div className="px-3 py-2.5 border-t border-[var(--color-border)] bg-[var(--color-bg-main)]">
        <div className="flex gap-2 items-end">
          <textarea
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t('chat.placeholder', lang)}
            rows={1}
            className="flex-1 bg-[var(--color-bg-sidebar)] rounded-xl px-3 py-2 text-[13px] outline-none resize-none max-h-24 placeholder-[var(--color-text-secondary)] border border-[var(--color-border)]"
          />
          <button
            onClick={sendMessage}
            disabled={loading || !input.trim()}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-xl px-4 py-2 text-[13px] font-medium transition-colors cursor-pointer text-[var(--color-accent-text)]"
          >
            {t('chat.send', lang)}
          </button>
        </div>
      </div>
    </div>
  )
}
