import { Navigate, Route, Routes } from 'react-router-dom'
import { ClientProvider } from './auth/ClientProvider'
import { RequireClient, UnpairedOnly } from './auth/guards'
import { ClientShell } from './components/ClientShell'
import { ConnectPage } from './pages/ConnectPage'
import { DevicesPage } from './pages/DevicesPage'
import { HomePage } from './pages/HomePage'
import { ModelsPage } from './pages/ModelsPage'
import { RecoveryPage } from './pages/RecoveryPage'
import { RoutingPage } from './pages/RoutingPage'
import { SettingsPage } from './pages/SettingsPage'
import { ToolsPage } from './pages/ToolsPage'
import { WelcomePage } from './pages/WelcomePage'

export default function App() {
  return (
    <ClientProvider>
      <Routes>
        <Route
          path="/welcome"
          element={
            <UnpairedOnly>
              <WelcomePage />
            </UnpairedOnly>
          }
        />
        <Route
          path="/connect"
          element={
            <UnpairedOnly>
              <ConnectPage />
            </UnpairedOnly>
          }
        />
        <Route element={<RequireClient />}>
          <Route element={<ClientShell />}>
            <Route index element={<HomePage />} />
            <Route path="tools" element={<ToolsPage />} />
            <Route path="models" element={<ModelsPage />} />
            <Route path="routing" element={<RoutingPage />} />
            <Route path="devices" element={<DevicesPage />} />
            <Route path="settings" element={<SettingsPage />} />
            <Route path="recovery" element={<RecoveryPage />} />
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </ClientProvider>
  )
}
