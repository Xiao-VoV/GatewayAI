import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { setActiveLocale, translate } from './translate'
import type { Locale, MessageKey, TranslateParams } from './translate'
import { SUPPORTED_LOCALES } from './translate'

interface I18nValue {
  locale: Locale
  setLocale: (locale: Locale) => void
  tr: (key: MessageKey, params?: TranslateParams) => string
}

const STORAGE_KEY = 'orveil-admin-locale'
const I18nContext = createContext<I18nValue | null>(null)

function readLocale(): Locale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY)
    return SUPPORTED_LOCALES.some((item) => item.id === stored) ? (stored as Locale) : 'zh-CN'
  } catch {
    return 'zh-CN'
  }
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(readLocale)

  const setLocale = useCallback((next: Locale) => setLocaleState(next), [])
  const tr = useCallback(
    (key: MessageKey, params?: TranslateParams) => translate(locale, key, params),
    [locale],
  )

  useEffect(() => {
    setActiveLocale(locale)
    document.documentElement.lang = locale
    try {
      localStorage.setItem(STORAGE_KEY, locale)
    } catch {
      // preference is non-critical
    }
  }, [locale])

  const value = useMemo<I18nValue>(() => ({ locale, setLocale, tr }), [locale, setLocale, tr])
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext)
  if (!value) throw new Error('useI18n must be used inside I18nProvider')
  return value
}
