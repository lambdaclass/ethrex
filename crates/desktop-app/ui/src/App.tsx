import { useState } from 'react'
import Sidebar from './components/Sidebar'
import ChatView from './components/ChatView'
import NodeControlView from './components/NodeControlView'
import DashboardView from './components/DashboardView'
import WalletView from './components/WalletView'
import SettingsView from './components/SettingsView'

export type ViewType = 'chat' | 'nodes' | 'dashboard' | 'wallet' | 'settings'

function App() {
  const [activeView, setActiveView] = useState<ViewType>('chat')

  const renderView = () => {
    switch (activeView) {
      case 'chat': return <ChatView />
      case 'nodes': return <NodeControlView />
      case 'dashboard': return <DashboardView />
      case 'wallet': return <WalletView />
      case 'settings': return <SettingsView />
    }
  }

  return (
    <div className="flex h-screen w-screen">
      <Sidebar activeView={activeView} onNavigate={setActiveView} />
      <main className="flex-1 flex flex-col overflow-hidden">
        {renderView()}
      </main>
    </div>
  )
}

export default App
