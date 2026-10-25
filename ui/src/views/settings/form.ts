import { h } from '../../lib/dom';

/** A labelled form row with an optional hint. */
export const field = (label: string, input: HTMLElement, hint?: string) => h('label.form-field', h('span.form-label', label), input, hint && h('span.form-hint', hint));
