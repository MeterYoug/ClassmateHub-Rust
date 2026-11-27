import { readonly, ref } from 'vue'
import en from 'element-plus/es/locale/lang/en.mjs'
import ja from 'element-plus/es/locale/lang/ja.mjs'
import ko from 'element-plus/es/locale/lang/ko.mjs'
import zhCn from 'element-plus/es/locale/lang/zh-cn.mjs'
import zhTw from 'element-plus/es/locale/lang/zh-tw.mjs'
import messages from './locales/messages.json' with { type: 'json' }

const defaultLocale = 'zh-Hans'
const storageKey = 'classmatehub.locale'
const elementLocales = { 'zh-Hans': zhCn, en, 'zh-Hant': zhTw, ja, ko }

export const languages = [
  { id: 'zh-Hans', name: '简体中文' },
  { id: 'en', name: 'English' },
  { id: 'zh-Hant', name: '繁體中文' },
  { id: 'ja', name: '日本語' },
  { id: 'ko', name: '한국어' },
]

function storedLocale() {
  try {
    const saved = globalThis.window?.localStorage.getItem(storageKey)
    if (Object.hasOwn(elementLocales, saved)) return saved
  } catch {
    // The window may restrict access to storage.
  }
  return defaultLocale
}

function updateDocumentLanguage(value) {
  if (globalThis.document?.documentElement) document.documentElement.lang = value
}

const current = ref(storedLocale())
export const locale = readonly(current)
updateDocumentLanguage(current.value)

export function setLocale(value) {
  if (!Object.hasOwn(elementLocales, value)) return
  current.value = value
  updateDocumentLanguage(value)
  try {
    globalThis.window?.localStorage.setItem(storageKey, value)
  } catch {
    // The language still applies to the current window.
  }
}

export function elementLocale() {
  return elementLocales[current.value]
}

const placeholder = /\{(\d+)\}/g
const escapePattern = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const messagePatterns = Object.keys(messages.en)
  .map(source => {
    const parts = source.split(placeholder)
    if (parts.length === 1) return null
    const literals = parts.filter((_, index) => index % 2 === 0)
    const fixedText = literals.join('')
    if (!/\p{Script=Han}/u.test(fixedText)) return null
    const parameters = parts.filter((_, index) => index % 2 === 1)
    return {
      source,
      parameters,
      specificity: fixedText.length,
      pattern: new RegExp('^' + literals.map(escapePattern).join('(.*?)') + '$', 's'),
    }
  })
  .filter(Boolean)
  .sort((a, b) => b.specificity - a.specificity)

const nestedMessages = {
  '第 {0} 行：{1}': 1,
  '第 {0} 行第 {1} 列：{2}': 2,
  '锁座冲突：{0}': 0,
  '恢复失败，已回退到恢复前的数据：{0}': 0,
  '恢复中断且回退未完成：{0}': 0,
  '读取布局失败：{0}': 0,
  '保存布局失败：{0}': 0,
  '保存失败：{0}': 0,
  '复制失败：{0}': 0,
  '{0}。已改用本地智能排座生成草稿': 0,
}

export function t(source, ...values) {
  const catalog = messages[current.value]
  const translation = catalog && Object.hasOwn(catalog, source) ? catalog[source] : source
  return values.length
    ? translation.replace(placeholder, (token, index) => String(values[index] ?? token))
    : translation
}

// Translate messages returned by the desktop commands or held by an open form.
// Arguments such as a student's name, a model identifier or a path are carried through as entered.
export function localizeMessage(message) {
  if (typeof message !== 'string' || current.value === defaultLocale) return message
  const catalog = messages[current.value]
  if (Object.hasOwn(catalog, message)) return catalog[message]
  for (const { source, parameters, pattern } of messagePatterns) {
    if (!Object.hasOwn(catalog, source)) continue
    const match = pattern.exec(message)
    if (!match) continue
    const values = []
    parameters.forEach((parameter, index) => { values[parameter] = match[index + 1] })
    const detail = Object.hasOwn(nestedMessages, source) ? nestedMessages[source] : null
    if (detail != null && values[detail]?.length < message.length) {
      values[detail] = localizeMessage(values[detail])
    }
    return t(source, ...values)
  }
  return message
}
