import { useI18n } from './I18nProvider'

export function LocaleSwitch({ className = '' }: { className?: string }) {
  const { locale, toggleLocale } = useI18n()
  return (
    <button
      className={`btn sm ghost locale-switch ${className}`.trim()}
      type="button"
      onClick={toggleLocale}
      title={locale === 'zh-CN' ? 'Switch to English' : '切换到中文'}
    >
      {locale === 'zh-CN' ? 'EN' : '中文'}
    </button>
  )
}
