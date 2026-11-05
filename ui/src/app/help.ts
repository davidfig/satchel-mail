import { api } from '../api';
import { h } from '../lib/dom';
import { errorToast, keyLabel, modal, shortcutList } from '../lib/ui';
import { applyTheme, store } from '../state';

export function toggleTheme() {
  const s = store.state.settings;
  if (!s) return;
  const dark = s.theme === 'dark' || (s.theme === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const next = { ...s, theme: dark ? 'light' : 'dark' };
  applyTheme(next.theme);
  void api.updateSettings(next).then((settings) => store.set({ settings })).catch(errorToast);
}

export function showShortcuts() {
  const seen = new Set<string>();
  const rows = shortcutList().filter((s) => {
    const k = `${s.key}|${s.desc}`;
    if (seen.has(k)) return false;
    seen.add(k);
    return !['ArrowDown', 'ArrowUp', 'Backspace', 'o'].includes(s.key);
  });
  modal('Keyboard shortcuts', h('table.shortcuts', h('tbody', rows.map((r) => h('tr', h('td', h('kbd', keyLabel(r.key))), h('td', r.desc), h('td.muted.small', r.scope))))));
}
