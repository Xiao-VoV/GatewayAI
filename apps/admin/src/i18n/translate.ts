import { enUS } from './messages/en-US'
import { zhCN } from './messages/zh-CN'

export const SUPPORTED_LOCALES = [
  { id: 'zh-CN', label: '中文' },
  { id: 'en-US', label: 'English' },
] as const

export type Locale = (typeof SUPPORTED_LOCALES)[number]['id']
export type MessageKey = keyof typeof zhCN

const catalogs: Record<Locale, Record<string, string>> = {
  'zh-CN': zhCN,
  'en-US': enUS,
}

export type TranslateParams = Record<string, string | number>
export type Translate = (key: MessageKey, params?: TranslateParams) => string

let activeLocale: Locale = 'zh-CN'

export function setActiveLocale(locale: Locale) {
  activeLocale = locale
}

function interpolate(template: string, params?: TranslateParams) {
  if (!params) return template
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name]
    return value === undefined ? match : String(value)
  })
}

export function translate(locale: Locale, key: MessageKey, params?: TranslateParams): string {
  const template = catalogs[locale]?.[key] ?? catalogs['zh-CN'][key] ?? key
  return interpolate(template, params)
}

export function translateActive(key: MessageKey, params?: TranslateParams): string {
  return translate(activeLocale, key, params)
}
