import { Navigate, Route, Routes } from 'react-router-dom'
import { AuthProvider } from './auth/AuthProvider'
import { LoginGate, RequireAuth, SetupGate } from './auth/guards'
import { AdminShell } from './components/AdminShell'
import { CatalogPreviewPage } from './pages/CatalogPreviewPage'
import { AccountPage } from './pages/AccountPage'
import { AuditPage } from './pages/AuditPage'
import { DevicesPage } from './pages/DevicesPage'
import { GroupsPage } from './pages/GroupsPage'
import { KeysPage } from './pages/KeysPage'
import { LogsPage } from './pages/LogsPage'
import { ModelDetailPage } from './pages/ModelDetailPage'
import { ModelsPage } from './pages/ModelsPage'
import { ProviderDetailPage } from './pages/ProviderDetailPage'
import { ProvidersPage } from './pages/ProvidersPage'
import { LoginPage } from './pages/LoginPage'
import { OverviewPage } from './pages/OverviewPage'
import { SetupPage } from './pages/SetupPage'
import { SettingsPage } from './pages/SettingsPage'
import { ToolDefaultsPage } from './pages/ToolDefaultsPage'

export default function App() {
  return (
    <AuthProvider>
      <Routes>
        <Route
          path="/setup"
          element={
            <SetupGate>
              <SetupPage />
            </SetupGate>
          }
        />
        <Route
          path="/login"
          element={
            <LoginGate>
              <LoginPage />
            </LoginGate>
          }
        />
        <Route element={<RequireAuth />}>
          <Route element={<AdminShell />}>
            <Route index element={<OverviewPage />} />
            <Route path="providers" element={<ProvidersPage />} />
            <Route path="providers/:providerId" element={<ProviderDetailPage />} />
            <Route path="models" element={<ModelsPage />} />
            <Route path="models/:groupId" element={<ModelDetailPage />} />
            <Route path="tool-defaults" element={<ToolDefaultsPage />} />
            <Route path="catalog-preview" element={<CatalogPreviewPage />} />
            <Route path="keys" element={<KeysPage />} />
            <Route path="devices" element={<DevicesPage />} />
            <Route path="groups" element={<GroupsPage />} />
            <Route path="logs" element={<LogsPage />} />
            <Route path="audit" element={<AuditPage />} />
            <Route path="settings" element={<SettingsPage />} />
            <Route path="account" element={<AccountPage />} />
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </AuthProvider>
  )
}
