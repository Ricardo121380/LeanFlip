// 只改一个键的 TOML 文本编辑：其余行（注释、顺序、空行）原样保留。
// 读取交给 smol-toml；写入按行定位表头和键，因为 Codex / Grok 的配置要“按原文保存”。
import { parse as parseToml } from "smol-toml";

/** 表路径，例如 ["model_providers", "custom"]；`null` 表示顶层。 */
export type TablePath = string[] | null;

const HEADER = /^\s*\[(?!\[)(.+)\]\s*(?:#.*)?$/;

function splitKey(raw: string): string[] {
  const parts: string[] = [];
  let current = "";
  let quote: string | null = null;
  for (const ch of raw) {
    if (quote) {
      if (ch === quote) quote = null;
      else current += ch;
    } else if (ch === '"' || ch === "'") {
      quote = ch;
    } else if (ch === ".") {
      parts.push(current.trim());
      current = "";
    } else {
      current += ch;
    }
  }
  parts.push(current.trim());
  return parts;
}

function sameTable(a: string[], b: string[]) {
  return a.length === b.length && a.every((part, index) => part === b[index]);
}

function keyLiteral(key: string) {
  return /^[A-Za-z0-9_-]+$/.test(key) ? key : JSON.stringify(key);
}

function tableHeader(table: string[]) {
  return `[${table.map(keyLiteral).join(".")}]`;
}

export function valueLiteral(value: string | number | boolean) {
  return typeof value === "string" ? JSON.stringify(value) : String(value);
}

/** 解析整段文本；格式有误时返回 null。 */
export function readToml(text: string): Record<string, unknown> | null {
  try {
    return parseToml(text) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export function getTomlValue(text: string, table: TablePath, key: string): unknown {
  const root = readToml(text);
  if (!root) return undefined;
  let node: unknown = root;
  for (const part of table ?? []) {
    node = node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined;
  }
  return node && typeof node === "object" ? (node as Record<string, unknown>)[key] : undefined;
}

// Track complete TOML statements so examples inside strings and arrays are never
// mistaken for keys or table headers. Ranges include a multiline value in full.
function statementRanges(lines: string[]): { start: number; end: number }[] {
  const ranges: { start: number; end: number }[] = [];
  let quote = "";
  let depth = 0;
  let start = -1;
  for (let row = 0; row < lines.length; row++) {
    const line = lines[row];
    if (start === -1 && !/^\s*(?:#|$)/.test(line)) start = row;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (quote) {
        if (quote[0] === '"' && ch === "\\") { i++; continue; }
        if (line.startsWith(quote, i)) {
          let length = quote.length;
          if (length === 3) while (line[i + length] === quote[0]) length++;
          i += length - 1;
          quote = "";
        }
      } else if (ch === "#") {
        break;
      } else if (ch === '"' || ch === "'") {
        quote = line.startsWith(ch.repeat(3), i) ? ch.repeat(3) : ch;
        i += quote.length - 1;
      } else if (ch === "[" || ch === "{") {
        depth++;
      } else if (ch === "]" || ch === "}") {
        depth--;
      }
    }
    if (start !== -1 && !quote && depth === 0) {
      ranges.push({ start, end: row + 1 });
      start = -1;
    }
  }
  return ranges;
}

/** 设置或删除（value 为空字符串 / undefined）一个键，返回新文本。 */
export function setTomlValue(text: string, table: TablePath, key: string, value: string | number | boolean | undefined): string {
  const lines = text.split("\n");
  const statements = statementRanges(lines);
  const remove = value === undefined || value === "";
  // Locate the table's line range [start, end).
  let start = 0;
  let end = lines.length;
  let headerIndex = -1;
  if (table) {
    const found = statements.find(({ start }) => {
      const match = HEADER.exec(lines[start]);
      return match ? sameTable(splitKey(match[1]), table) : false;
    })?.start ?? -1;
    if (found === -1) {
      if (remove) return text;
      const body = text.endsWith("\n") || text === "" ? text : `${text}\n`;
      const gap = body.trim() ? "\n" : "";
      return `${body}${gap}${tableHeader(table)}\n${keyLiteral(key)} = ${valueLiteral(value!)}\n`;
    }
    headerIndex = found;
    start = found + 1;
  }
  const next = statements.find((range) => range.start >= start && (HEADER.test(lines[range.start]) || /^\s*\[\[/.test(lines[range.start])))?.start ?? -1;
  if (next !== -1) end = next;

  const keyPattern = new RegExp(`^\\s*(?:${escapeRegExp(key)}|"${escapeRegExp(key)}"|'${escapeRegExp(key)}')\\s*=`);
  const existing = statements.find((range) => range.start >= start && range.start < end && keyPattern.test(lines[range.start]));
  if (existing) {
    if (remove) lines.splice(existing.start, existing.end - existing.start);
    else {
      const indent = /^\s*/.exec(lines[existing.start])![0];
      lines.splice(existing.start, existing.end - existing.start, `${indent}${keyLiteral(key)} = ${valueLiteral(value!)}`);
    }
    return lines.join("\n");
  }
  if (remove) return text;
  // Insert after the last key line in the table, so new keys sit with their siblings.
  let insertAt = table ? headerIndex + 1 : 0;
  for (const range of statements) {
    if (range.start >= start && range.start < end && /^\s*[^#\s[].*=/.test(lines[range.start])) insertAt = range.end;
  }
  lines.splice(insertAt, 0, `${keyLiteral(key)} = ${valueLiteral(value!)}`);
  return lines.join("\n");
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
