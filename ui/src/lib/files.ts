// Which attachments the viewer can show, and how (see views/mail/attachments.ts, 0054).

import type { Attachment } from '../types';

export type FileKind = 'image' | 'pdf' | 'audio' | 'video' | 'csv' | 'text';

const IMAGE = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'avif', 'svg', 'heic', 'heif', 'tif', 'tiff', 'ico'];
const AUDIO = ['mp3', 'm4a', 'aac', 'wav', 'ogg', 'oga', 'opus', 'flac'];
const VIDEO = ['mp4', 'm4v', 'mov', 'webm', 'ogv'];
const TEXT = ['txt', 'text', 'md', 'markdown', 'log', 'json', 'xml', 'yaml', 'yml', 'toml', 'ini', 'conf', 'cfg', 'ics', 'vcf', 'eml', 'diff', 'patch', 'sql', 'css', 'rs', 'ts', 'js', 'py', 'go', 'rb', 'java', 'c', 'h', 'cpp', 'swift', 'kt', 'sh'];
const IMAGE_TYPES: Record<string, string> = { jpg: 'image/jpeg', svg: 'image/svg+xml', tif: 'image/tiff', ico: 'image/x-icon' };

function extension(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot < 0 ? '' : name.slice(dot + 1).toLowerCase();
}

/** How the viewer shows this attachment, or null if it can't. Risky files
 *  (HTML pages, programs) are never shown, whatever they claim to be. */
export function previewKind(a: Pick<Attachment, 'filename' | 'mime_type' | 'risky'>): FileKind | null {
  if (a.risky) return null;
  const mime = a.mime_type.toLowerCase().split(';')[0].trim();
  const ext = extension(a.filename);
  if (mime === 'text/html' || ext === 'html' || ext === 'htm') return null;
  if (mime === 'application/pdf' || ext === 'pdf') return 'pdf';
  if (mime.startsWith('image/') || IMAGE.includes(ext)) return 'image';
  if (mime.startsWith('audio/') || AUDIO.includes(ext)) return 'audio';
  if (mime.startsWith('video/') || VIDEO.includes(ext)) return 'video';
  if (mime === 'text/csv' || mime === 'text/tab-separated-values' || ext === 'csv' || ext === 'tsv') return 'csv';
  if (mime.startsWith('text/') || mime === 'application/json' || mime === 'application/xml' || TEXT.includes(ext)) return 'text';
  return null;
}

/** The type a blob is given: never one the webview would run, only the kind we chose to show. */
export function blobType(kind: FileKind, a: Attachment): string {
  const mime = a.mime_type.toLowerCase().split(';')[0].trim();
  if (kind === 'pdf') return 'application/pdf';
  const ext = extension(a.filename);
  if (kind === 'image') return mime.startsWith('image/') ? mime : (IMAGE_TYPES[ext] ?? `image/${ext}`);
  if (kind === 'audio' || kind === 'video') return mime.startsWith(`${kind}/`) ? mime : `${kind}/${ext === 'mov' ? 'quicktime' : ext}`;
  return 'text/plain';
}

/** A CSV (or TSV) file as a table, quoted fields and all (RFC 4180). */
export function parseCsv(text: string, sep: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') quoted = false;
      else field += c;
    } else if (c === '"' && field === '') quoted = true;
    else if (c === sep) {
      row.push(field);
      field = '';
    } else if (c === '\n' || c === '\r') {
      if (c === '\r' && text[i + 1] === '\n') i++;
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else field += c;
  }
  if (field || row.length) rows.push([...row, field]);
  return rows;
}

