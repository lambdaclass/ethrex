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
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      sendMessage()
    }
  }

  return (
    <div className="flex flex-col h-full bg-[var(--color-bg-chat)]">
      <div className="px-6 py-4 border-b border-[var(--color-border)] flex items-center justify-between">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-full bg-[var(--color-accent)] flex items-center justify-center">🤖</div>
          <div>
            <div className="font-semibold">{t('chat.title', lang)}</div>
            <div className="text-xs text-[var(--color-text-secondary)]">{t('chat.notConnected', lang)}</div>
          </div>
        </div>
        <select className="bg-[var(--color-border)] text-sm rounded-lg px-3 py-1.5 outline-none">
          <option>Claude</option>
          <option>GPT</option>
          <option>Gemini</option>
        </select>
      </div>

      <div className="flex-1 overflow-y-auto px-6 py-4 space-y-4">
        {messages.map((msg, i) => (
          <div key={i} className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div
              className={`max-w-[70%] rounded-2xl px-4 py-3 text-sm whitespace-pre-wrap leading-relaxed ${
                msg.role === 'user'
                  ? 'bg-[var(--color-bubble-user)] rounded-br-md'
                  : 'bg-[var(--color-bubble-ai)] rounded-bl-md'
              }`}
            >
              {msg.content}
            </div>
          </div>
        ))}
        {loading && (
          <div className="flex justify-start">
            <div className="bg-[var(--color-bubble-ai)] rounded-2xl rounded-bl-md px-4 py-3 text-sm">
              <span className="animate-pulse">{t('chat.thinking', lang)}</span>
            </div>
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      <div className="px-6 py-4 border-t border-[var(--color-border)]">
        <div className="flex gap-3 items-end">
          <textarea
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t('chat.placeholder', lang)}
            rows={1}
            className="flex-1 bg-[var(--color-border)] rounded-xl px-4 py-3 text-sm outline-none resize-none max-h-32 placeholder-[var(--color-text-secondary)]"
          />
          <button
            onClick={sendMessage}
            disabled={loading || !input.trim()}
            className="bg-[var(--color-accent)] hover:bg-[var(--color-accent-hover)] disabled:opacity-40 rounded-xl px-5 py-3 text-sm font-medium transition-colors cursor-pointer"
          >
            {t('chat.send', lang)}
          </button>
        </div>
      </div>
    </div>
  )
}
