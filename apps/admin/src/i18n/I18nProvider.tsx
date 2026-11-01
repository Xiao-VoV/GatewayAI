import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { setApiLocale } from '../api/client'

export type Locale = 'zh-CN' | 'en-US'

interface I18nValue {
  locale: Locale
  setLocale: (locale: Locale) => void
  toggleLocale: () => void
  tr: (zh: string, en: string) => string
}

const STORAGE_KEY = 'orveil-admin-locale'
const I18nContext = createContext<I18nValue | null>(null)

function readLocale(): Locale {
  try {
    return localStorage.getItem(STORAGE_KEY) === 'en-US' ? 'en-US' : 'zh-CN'
  } catch {
    return 'zh-CN'
  }
}

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(readLocale)

  const setLocale = useCallback((next: Locale) => setLocaleState(next), [])
  const toggleLocale = useCallback(
    () => setLocaleState((current) => (current === 'zh-CN' ? 'en-US' : 'zh-CN')),
    [],
  )
  const tr = useCallback((zh: string, en: string) => (locale === 'en-US' ? en : zh), [locale])

  useEffect(() => {
    setApiLocale(locale)
    document.documentElement.lang = locale
    try {
      localStorage.setItem(STORAGE_KEY, locale)
    } catch {
      // preference is non-critical
    }
  }, [locale])

  const value = useMemo<I18nValue>(
    () => ({ locale, setLocale, toggleLocale, tr }),
    [locale, setLocale, toggleLocale, tr],
  )
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext)
  if (!value) throw new Error('useI18n must be used inside I18nProvider')
  return value
}
