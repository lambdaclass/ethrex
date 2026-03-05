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
import type { Lang } from './i18n'
import type { NetworkMode } from './components/CreateL2Wizard'

export type ViewType = 'home' | 'myl2' | 'chat' | 'nodes' | 'dashboard' | 'openl2' | 'wallet' | 'settings'
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
  const [createNetwork, setCreateNetwork] = useState<NetworkMode | undefined>()
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
      case 'myl2': return <MyL2View initialNetwork={createNetwork} onNetworkConsumed={() => setCreateNetwork(undefined)} />
      case 'chat': return <ChatView />
      case 'nodes': return <NodeControlView />
      case 'dashboard': return <DashboardView />
      case 'openl2': return <OpenL2View />
      case 'wallet': return <WalletView />
      case 'settings': return <SettingsView />
    }
  }

  return (
    <AppContext.Provider value={{ lang, setLang: handleSetLang, theme, setTheme: handleSetTheme }}>
      <div className="flex h-screen w-screen">
        <Sidebar activeView={activeView} onNavigate={navigateTo} />
        <main className="flex-1 flex flex-col overflow-hidden">
          {renderView()}
        </main>
      </div>
    </AppContext.Provider>
  )
}

export default App
