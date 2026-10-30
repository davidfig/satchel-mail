// Settings: accounts, AI agents, AI folder, privacy, appearance, activity.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { errorToast } from '../../lib/ui';
import { store } from '../../state';
import type { Settings } from '../../types';
import { accounts } from './accounts';
import { agents } from './agents';
import { activity, general, mirror } from './general';
import { privacy } from './privacy';

type Section = 'accounts' | 'agents' | 'mirror' | 'privacy' | 'general' | 'activity';

export class SettingsView {
  root = h('div.settings-view');
  private section: Section = 'accounts';

  /** The user has typed into a form here since it was drawn. */
  private editing = false;

  constructor() {
    // Redraw for news, never over unsaved edits. Account changes arrive
    // constantly during a sync; only the Accounts page shows them.
    store.subscribe((_, changed) => {
      if (store.state.view !== 'settings' || this.editing) return;
      if ((changed.has('accounts') && this.section === 'accounts') || changed.has('settings')) this.render();
    });
    this.root.addEventListener('input', () => (this.editing = true));
    this.root.addEventListener('change', () => (this.editing = true));
  }

  show() {
    this.render();
  }

  render() {
    this.editing = false;
    const nav: [Section, string, string][] = [
      ['accounts', 'Accounts', 'user'],
      ['agents', 'AI agents', 'sparkle'],
      ['mirror', 'AI folder', 'folder'],
      ['privacy', 'Privacy', 'shield'],
      ['general', 'General', 'settings'],
      ['activity', 'Activity', 'clock'],
    ];
    const content = h('div.settings-content');
    replace(this.root,
      h('nav.settings-nav', nav.map(([s, l, i]) => h(`button.nav-item${this.section === s ? '.active' : ''}`, { on: { click: () => { this.section = s; this.render(); } } }, icon(i), l))),
      content);
    const r = { accounts: () => accounts(content), agents: () => agents(this, content), mirror: () => mirror(this, content), privacy: () => privacy(this, content), general: () => general(this, content), activity: () => activity(this, content) };
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
