import ru from './ru.json';
import en from './en.json';

export type Locale = 'ru' | 'en';
export type LocaleSetting = Locale | 'auto';

const dictionaries: Record<Locale, Record<string, string>> = { ru, en };

export const locales: { id: LocaleSetting; label: string }[] = [
  { id: 'auto', label: 'Как в системе' },
  { id: 'ru', label: 'Русский' },
  { id: 'en', label: 'English' },
];

let current = $state<Locale>('ru');

export const locale = {
  get current() {
    return current;
  },
};

export function setLocale(next: LocaleSetting) {
  current = next === 'auto' ? detectLocale() : next;
  document.documentElement.lang = current;
}

export function detectLocale(): Locale {
  const lang = navigator.language.toLowerCase();
  return lang.startsWith('ru') ? 'ru' : 'en';
}

export function t(key: string): string {
  return dictionaries[current][key] ?? dictionaries.ru[key] ?? key;
}
