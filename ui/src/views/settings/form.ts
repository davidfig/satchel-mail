import { h } from '../../lib/dom';
import type { Security } from '../../types';

/** A labelled form row with an optional hint. */
export const field = (label: string, input: HTMLElement, hint?: string) => h('label.form-field', h('span.form-label', label), input, hint && h('span.form-hint', hint));

/** Connection security for a mail server. */
export function secSelect(v: Security) {
  return h('select', [['tls', 'SSL/TLS'], ['start_tls', 'STARTTLS'], ['none', 'None']].map(([val, l]) => h('option', { value: val, selected: v === val }, l)));
}
