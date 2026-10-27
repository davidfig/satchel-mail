// Settings → AI agents: connected MCP agents and their access.

import { api } from '../../api';
import { h, icon, replace } from '../../lib/dom';
import { relative } from '../../lib/format';
import { confirmDialog, errorToast, modal, toast } from '../../lib/ui';
import { store } from '../../state';
import { transport } from '../../transport';
import type { Agent, AgentPolicy } from '../../types';
import { field } from './form';
import type { SettingsView } from './index';

export async function agents(view: SettingsView, el: HTMLElement) {
  let agents: Agent[] = [];
  let info: Awaited<ReturnType<typeof api.info>> | null = null;
  try {
    [agents, info] = await Promise.all([api.listAgents(), api.info()]);
  } catch (e) {
    errorToast(e);
  }
  const active = agents.filter((a) => !a.revoked && !a.builtin);
  replace(el,
    h('h1', 'AI agents'),
    h('p.muted', 'Most AI tools work through the AI folder and need no setup here. Connect an agent for tools that use MCP, or to give one tool its own access limits. Every proposal waits in Review until you approve it. Agents can never send, delete or change settings.'),
    h('div.card-list', active.map((a) =>
      h('div.account-card',
        icon('sparkle'),
        h('div.grow',
          h('strong', a.name),
          h('div.small.muted', `Created ${relative(a.created_at)} · last used ${relative(a.last_used)}`),
          h('div.small.muted', policyLabel(a.policy))),
        h('button', { title: 'Change what this agent can read and propose', on: { click: () => editAgent(view, a) } }, 'Edit access'),
        h('button.danger', { title: 'Cut off this agent’s access', on: { click: async () => { if (await confirmDialog('Revoke agent?', `${a.name} will lose access immediately.`, 'Revoke', true)) { await api.revokeAgent(a.id); view.render(); } } } }, 'Revoke')))),
    h('button.primary', { on: { click: () => editAgent(view, null, info?.data_dir) } }, icon('plus', 16), 'Connect an agent'),
  );
}

export function editAgent(view: SettingsView, existing: Agent | null, dataDir?: string) {
  const pol: AgentPolicy = existing?.policy ?? { account_ids: null, since_days: 365, allow_attachments: false, allow_calendar: true, allow_contacts: true, max_proposals_per_hour: 60 };
  const name = h('input', { type: 'text', value: existing?.name ?? 'Claude', placeholder: 'e.g. Claude Code' });
  const acctBoxes = store.state.accounts.map((a) => ({ a, box: h('input', { type: 'checkbox', checked: !pol.account_ids || pol.account_ids.includes(a.id) }) }));
  const since = h('select', [['30', 'Last 30 days'], ['90', 'Last 90 days'], ['365', 'Last year'], ['', 'Everything']].map(([v, l]) => h('option', { value: v, selected: String(pol.since_days ?? '') === v }, l)));
  const att = h('input', { type: 'checkbox', checked: pol.allow_attachments });
  const cal = h('input', { type: 'checkbox', checked: pol.allow_calendar });
  const contacts = h('input', { type: 'checkbox', checked: pol.allow_contacts ?? pol.allow_calendar });
  const rate = h('input', { type: 'number', value: String(pol.max_proposals_per_hour), min: '1', max: '1000' });
  const save = async () => {
    const ids = acctBoxes.filter((x) => x.box.checked).map((x) => x.a.id);
    const policy: AgentPolicy = {
      account_ids: ids.length === store.state.accounts.length ? null : ids,
      since_days: since.value ? Number(since.value) : null,
      allow_attachments: att.checked,
      allow_calendar: cal.checked,
      allow_contacts: contacts.checked,
      max_proposals_per_hour: Number(rate.value) || 60,
    };
    try {
      if (existing) {
        await api.updateAgent(existing.id, name.value.trim() || existing.name, policy);
        m.close();
        view.render();
        return;
      }
      const r = await api.createAgent(name.value.trim() || 'Agent', policy);
      m.close();
      void showToken(r.token, dataDir);
      view.render();
    } catch (e) {
      errorToast(e);
    }
  };
  const m = modal(existing ? `Access for ${existing.name}` : 'Connect an AI agent', h('div.form',
    field('Name', name),
    h('fieldset', h('legend', 'Accounts it can read'), acctBoxes.map(({ a, box }) => h('label.row.inline', box, a.email))),
    field('Mail history', since),
    h('label.row.inline', cal, 'Can read calendars and propose events'),
    h('label.row.inline', contacts, 'Can read contacts and propose changes to them'),
    h('label.row.inline', att, 'Can read binary attachments (text attachments are always readable)'),
    field('Max proposals per hour', rate)), {
    footer: h('div.row.end', h('button', { on: { click: () => m.close() } }, 'Cancel'), h('button.primary', { on: { click: () => void save() } }, existing ? 'Save' : 'Create token')),
  });
}

export async function showToken(token: string, dataDir?: string) {
  // The desktop app bundles the CLI; use its absolute path so it works without PATH changes.
  const cli = transport.kind === 'tauri' ? ((await transport.host<string | null>('cli_path').catch(() => null)) ?? 'satchel') : 'satchel';
  const quoted = cli.includes(' ') ? `"${cli}"` : cli;
  const env = dataDir && transport.kind !== 'tauri' ? ` --env SATCHEL_DATA_DIR="${dataDir}"` : '';
  const claude = `claude mcp add satchel --env SATCHEL_AGENT_TOKEN=${token}${env} -- ${quoted} mcp`;
  const json = JSON.stringify({ mcpServers: { satchel: { command: cli, args: ['mcp'], env: { SATCHEL_AGENT_TOKEN: token, ...(env && dataDir ? { SATCHEL_DATA_DIR: dataDir } : {}) } } } }, null, 2);
  const copy = (s: string) => () => void navigator.clipboard.writeText(s).then(() => toast('Copied'));
  modal('Agent token created', h('div.form',
    h('p', 'Copy this token now — it won’t be shown again.'),
    h('div.copy-row', h('code.token', token), h('button', { title: 'Copy to clipboard', on: { click: copy(token) } }, 'Copy')),
    h('h3', 'Claude Code'),
    h('div.copy-row', h('pre.snippet', claude), h('button', { title: 'Copy to clipboard', on: { click: copy(claude) } }, 'Copy')),
    h('h3', 'Claude Desktop / other MCP clients'),
    h('div.copy-row', h('pre.snippet', json), h('button', { title: 'Copy to clipboard', on: { click: copy(json) } }, 'Copy')),
    h('p.small.muted', 'Tools without MCP can use the AI folder instead; to give a proposal file this agent’s identity and limits, include the token as agent_token (see README.md there).')), { wide: true });
}

function policyLabel(p: AgentPolicy) {
  const accts = p.account_ids ? `${p.account_ids.length} account(s)` : 'all accounts';
  const hist = p.since_days ? `last ${p.since_days} days` : 'all history';
  return `${accts} · ${hist}${p.allow_calendar ? ' · calendars' : ''}${p.allow_contacts ? ' · contacts' : ''}${p.allow_attachments ? ' · attachments' : ''}`;
}
