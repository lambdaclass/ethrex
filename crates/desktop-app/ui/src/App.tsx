import { useState, useEffect, createContext, useContext } from 'react'
import Sidebar from './components/Sidebar'
import ChatView from './components/ChatView'
import NodeControlView from './components/NodeControlView'
import DashboardView from './components/DashboardView'
import WalletView from './components/WalletView'
import SettingsView from './components/SettingsView'
import OpenL2View from './components/OpenL2View'
import MyL2View from './components/MyL2View'
import HomeView from './components/HomeView'
import ProgramStoreView from './components/ProgramStoreView'
import type { Lang } from './i18n'
import type { NetworkMode } from './components/CreateL2Wizard'

export type ViewType = 'home' | 'myl2' | 'chat' | 'nodes' | 'dashboard' | 'openl2' | 'wallet' | 'store' | 'settings'
export type Theme = 'light' | 'dark'

interface AppContextType {
  lang: Lang
  setLang: (lang: Lang) => void
  theme: Theme
  setTheme: (theme: Theme) => void
}

export const AppContext = createContext<AppContextType>({ lang: 'ko', setLang: () => {}, theme: 'light', setTheme: () => {} })
export const useLang = () => useContext(AppContext)
export const useTheme = () => useContext(AppContext)

function App() {
  const [activeView, setActiveView] = useState<ViewType>('home')
  const [_createNetwork, setCreateNetwork] = useState<NetworkMode | undefined>()
  const [lang, setLang] = useState<Lang>(() => {
    return (localStorage.getItem('tokamak-lang') as Lang) || 'ko'
  })
  const [theme, setTheme] = useState<Theme>(() => {
    return (localStorage.getItem('tokamak-theme') as Theme) || 'light'
  })

  useEffect(() => {
    document.documentElement.classList.toggle('dark', theme === 'dark')
  }, [theme])

  const handleSetLang = (newLang: Lang) => {
    setLang(newLang)
    localStorage.setItem('tokamak-lang', newLang)
  }

  const handleSetTheme = (newTheme: Theme) => {
    setTheme(newTheme)
    localStorage.setItem('tokamak-theme', newTheme)
  }

  const navigateTo = (view: ViewType) => {
    setCreateNetwork(undefined)
    setActiveView(view)
  }

  const navigateToCreate = (network: NetworkMode) => {
    setCreateNetwork(network)
    setActiveView('myl2')
  }

  const renderView = () => {
    switch (activeView) {
      case 'home': return <HomeView onNavigate={navigateTo} onCreateWithNetwork={navigateToCreate} />
      case 'myl2': return <MyL2View />
      case 'nodes': return <NodeControlView />
      case 'dashboard': return <DashboardView />
      case 'openl2': return <OpenL2View />
      case 'wallet': return <WalletView />
      case 'store': return <ProgramStoreView />
      case 'settings': return <SettingsView />
      default: return null
    }
  }

  return (
    <AppContext.Provider value={{ lang, setLang: handleSetLang, theme, setTheme: handleSetTheme }}>
      <div className="flex h-screen w-screen">
        <Sidebar activeView={activeView} onNavigate={navigateTo} />
        <main className="flex-1 flex flex-col overflow-hidden">
          {/* ChatView is always mounted to preserve chat history and state */}
          <div className={activeView === 'chat' ? 'flex flex-col h-full' : 'hidden'}>
            <ChatView onNavigate={navigateTo} onCreateWithNetwork={navigateToCreate} isVisible={activeView === 'chat'} />
          </div>
          {activeView !== 'chat' && renderView()}
        </main>
      </div>
    </AppContext.Provider>
  )
}

export default App
