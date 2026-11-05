// What to tell an AI tool about Satchel: shown in AI Proposals and Settings → AI agents.

import { h } from '../lib/dom';
import { copyBlock } from '../lib/ui';
import { store } from '../state';

/** A prompt to paste into any AI tool with file access, or how to turn the
 *  AI folder on. `close` (a button that hides it) goes beside the first line. */
export function agentPromptBlock(root: string, note?: string, close?: Node): HTMLElement {
  const head = (text: string) => (close ? h('div.get-started-head', h('p.small', text), close) : h('p.small', text));
  const mi = store.state.settings?.mirror;
  if (mi && (!mi.enabled || !mi.accept_proposals)) {
    const off = `To let AI tools propose changes, turn on the AI folder and proposal files in Settings → AI folder.${note ? ` ${note}` : ''}`;
    return close ? h('div.get-started', head(off)) : h('p.muted.small', off);
  }
  return h('div.get-started',
    head('Paste this into an AI tool that can read and write files on this computer (Claude Code, Codex, Cursor and similar). It needs no setup.'),
    copyBlock(agentPrompt(root)),
    note && h('p.muted.small', note));
}

/** Instructions for an AI tool with file access; the folder's README has the details. */
function agentPrompt(root: string): string {
  const sep = root.includes('\\') ? '\\' : '/';
  const at = (p: string) => `${root}${sep}${p.replaceAll('/', sep)}`;
  return `You can help me with my email, calendar and contacts through satchel.email. It keeps an up-to-date, plain-text copy of them in this folder:

${root}

Start by reading ${at('README.md')}: it explains the layout and how to propose changes. Read and search the files directly. Editing the mail or calendar files changes nothing; they are a copy.

To act (reply, draft a message, archive, move, label, add or change an event, save a contact), write a proposal file to ${at('proposals/inbox/')} as the README describes, with a short "why:" and "agent: <your name>" in its header. I review every proposal in Satchel before anything happens. Check ${at('proposals/processed/')} to see what became of yours.

Never try to send mail or change my accounts any other way.`;
}
