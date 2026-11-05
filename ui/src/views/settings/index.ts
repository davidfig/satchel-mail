// Settings: accounts, AI agents, AI folder, privacy, favorites, appearance, activity.

import { api } from '../../api';
import { clear, h, icon, patch } from '../../lib/dom';
import { errorToast } from '../../lib/ui';
import { store } from '../../state';
import type { SettingsSection as Section } from '../../state';
import type { Settings } from '../../types';
import { accounts } from './accounts';
import { agents } from './agents';
import { favorites } from './favorites';
import { activity, general, mirror } from './general';
import { privacy } from './privacy';

export class SettingsView {
  private nav = h('nav.settings-nav');
  private content = h('div.settings-content');
  root = h('div.settings-view', this.nav, this.content);
  private section: Section = 'accounts';
  /** The section `content` currently holds. */
  private drawn: Section | null = null;

  /** The user has typed into a form here since it was drawn. */
  private editing = false;

  constructor() {
    // Redraw for news, never over unsaved edits. Account changes arrive
    // constantly during a sync; only the Accounts page shows them.
    store.subscribe((_, changed) => {
      if (store.state.view !== 'settings' || this.editing) return;
      // A link (or the palette) asking for a section while Settings is open.
      if (changed.has('settingsSection') && store.state.settingsSection) return this.show();
      if ((changed.has('accounts') && this.section === 'accounts') || (changed.has('favorites') && this.section === 'favorites') || changed.has('settings')) this.render();
    });
    this.root.addEventListener('input', () => (this.editing = true));
    this.root.addEventListener('change', () => (this.editing = true));
  }

  show() {
    const wanted = store.state.settingsSection;
    if (wanted) {
      this.section = wanted;
      store.set({ settingsSection: null });
    }
    this.render();
  }

  render() {
    this.editing = false;
    const nav: [Section, string, string][] = [
      ['accounts', 'Accounts', 'user'],
      ['agents', 'AI agents', 'sparkle'],
      ['mirror', 'AI folder', 'folder'],
      ['privacy', 'Privacy', 'shield'],
      ['favorites', 'Favorites', 'heart'],
      ['general', 'General', 'settings'],
      ['activity', 'Activity', 'clock'],
    ];
    // Patched, not replaced: the Accounts page redraws on every sync tick,
    // and swapping the nodes out made it blink. A new section starts clean,
    // since patching keeps the old nodes' DOM properties (input values, …).
    patch(this.nav, nav.map(([s, l, i]) => h(`button.nav-item${this.section === s ? '.active' : ''}`, { on: { click: () => { this.section = s; this.render(); } } }, icon(i), l)));
    const content = this.content;
    if (this.drawn !== this.section) clear(content);
    this.drawn = this.section;
    const r = { accounts: () => accounts(content), agents: () => agents(this, content), mirror: () => mirror(this, content), privacy: () => privacy(this, content), favorites: () => favorites(this, content), general: () => general(this, content), activity: () => activity(this, content) };
    void r[this.section]();
  }

  /** Saves settings; shows any error and returns whether it worked. */
  async saveSettings(next: Settings): Promise<boolean> {
    try {
      const saved = await api.updateSettings(next);
      // Saved: what's on screen is no longer an unsaved edit.
      this.editing = false;
      store.set({ settings: saved });
      return true;
    } catch (e) {
      errorToast(e);
      return false;
    }
  }
}
