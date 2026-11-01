import { SUPPORTED_LOCALES } from './translate'
import { useI18n } from './I18nProvider'

export function LocaleSwitch({ className = '' }: { className?: string }) {
  const { locale, setLocale } = useI18n()
  return (
    <select
      className={`select locale-select ${className}`.trim()}
      aria-label="Language"
      value={locale}
      onChange={(event) => setLocale(event.target.value as typeof locale)}
    >
      {SUPPORTED_LOCALES.map((item) => (
        <option key={item.id} value={item.id}>
          {item.label}
        </option>
      ))}
    </select>
  )
}
