import { useState, createContext, useContext } from 'react'
import Sidebar from './components/Sidebar'
import ChatView from './components/ChatView'
import NodeControlView from './components/NodeControlView'
import DashboardView from './components/DashboardView'
import WalletView from './components/WalletView'
import SettingsView from './components/SettingsView'
import OpenL2View from './components/OpenL2View'
import MyL2View from './components/MyL2View'
import type { Lang } from './i18n'

export type ViewType = 'myl2' | 'chat' | 'nodes' | 'dashboard' | 'openl2' | 'wallet' | 'settings'

interface LangContextType {
  lang: Lang
  setLang: (lang: Lang) => void
}

export const LangContext = createContext<LangContextType>({ lang: 'ko', setLang: () => {} })
export const useLang = () => useContext(LangContext)

function App() {
  const [activeView, setActiveView] = useState<ViewType>('myl2')
  const [lang, setLang] = useState<Lang>(() => {
    const saved = localStorage.getItem('tokamak-lang')
    return (saved as Lang) || 'ko'
  })

  const handleSetLang = (newLang: Lang) => {
    setLang(newLang)
    localStorage.setItem('tokamak-lang', newLang)
  }

  const renderView = () => {
    switch (activeView) {
      case 'myl2': return <MyL2View />
      case 'chat': return <ChatView />
      case 'nodes': return <NodeControlView />
      case 'dashboard': return <DashboardView />
      case 'openl2': return <OpenL2View />
      case 'wallet': return <WalletView />
      case 'settings': return <SettingsView />
    }
  }

  return (
    <LangContext.Provider value={{ lang, setLang: handleSetLang }}>
      <div className="flex h-screen w-screen">
        <Sidebar activeView={activeView} onNavigate={setActiveView} />
        <main className="flex-1 flex flex-col overflow-hidden">
          {renderView()}
        </main>
      </div>
    </LangContext.Provider>
  )
}

export default App
