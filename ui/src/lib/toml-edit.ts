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

export function valueLiteral(value: string | number) {
  return typeof value === "number" ? String(value) : JSON.stringify(value);
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

/** 设置或删除（value 为空字符串 / undefined）一个键，返回新文本。 */
export function setTomlValue(text: string, table: TablePath, key: string, value: string | number | undefined): string {
  const lines = text.split("\n");
  const remove = value === undefined || value === "";
  // Locate the table's line range [start, end).
  let start = 0;
  let end = lines.length;
  let headerIndex = -1;
  if (table) {
    const found = lines.findIndex((line) => {
      const match = HEADER.exec(line);
      return match ? sameTable(splitKey(match[1]), table) : false;
    });
    if (found === -1) {
      if (remove) return text;
      const body = text.endsWith("\n") || text === "" ? text : `${text}\n`;
      const gap = body.trim() ? "\n" : "";
      return `${body}${gap}${tableHeader(table)}\n${keyLiteral(key)} = ${valueLiteral(value!)}\n`;
    }
    headerIndex = found;
    start = found + 1;
  }
  const next = lines.findIndex((line, index) => index >= start && (HEADER.test(line) || /^\s*\[\[/.test(line)));
  if (next !== -1) end = next;

  const keyPattern = new RegExp(`^\\s*(?:${escapeRegExp(key)}|"${escapeRegExp(key)}"|'${escapeRegExp(key)}')\\s*=`);
  const existing = lines.findIndex((line, index) => index >= start && index < end && keyPattern.test(line));
  if (existing !== -1) {
    if (remove) lines.splice(existing, 1);
    else {
      const indent = /^\s*/.exec(lines[existing])![0];
      lines[existing] = `${indent}${keyLiteral(key)} = ${valueLiteral(value!)}`;
    }
    return lines.join("\n");
  }
  if (remove) return text;
  // Insert after the last key line in the table, so new keys sit with their siblings.
  let insertAt = table ? headerIndex + 1 : 0;
  for (let index = start; index < end; index++) {
    if (/^\s*[^#\s[].*=/.test(lines[index])) insertAt = index + 1;
  }
  lines.splice(insertAt, 0, `${keyLiteral(key)} = ${valueLiteral(value!)}`);
  return lines.join("\n");
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
