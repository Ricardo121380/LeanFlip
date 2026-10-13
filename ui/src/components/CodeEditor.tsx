import { useEffect, useRef, useState } from "react";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { json } from "@codemirror/lang-json";
import { StreamLanguage, syntaxHighlighting, defaultHighlightStyle, bracketMatching, HighlightStyle } from "@codemirror/language";
import { toml } from "@codemirror/legacy-modes/mode/toml";

// Reuse CodeMirror's tag selection, but take colors from theme variables so both themes stay legible.
const TOKEN_COLORS: Record<string, string> = {
  "#708": "--code-keyword",
  "#219": "--code-number",
  "#164": "--code-number",
  "#a11": "--code-string",
  "#e40": "--code-string",
  "#940": "--code-comment",
  "#404740": "--code-comment",
  "#f00": "--destructive",
};
const themedHighlight = HighlightStyle.define(
  defaultHighlightStyle.specs.map((spec) =>
    typeof spec.color === "string" ? { ...spec, color: `hsl(var(${TOKEN_COLORS[spec.color] ?? "--code-key"}))` } : spec,
  ),
);

interface Props {
  value: string;
  onChange: (value: string) => void;
  language: "json" | "toml";
  minHeight?: number;
  ariaLabel?: string;
  readOnly?: boolean;
}

/** 纯文本编辑器：内容原样进出，不做任何格式化（TOML 一字不改地保存）。 */
export function CodeEditor({ value, onChange, language, minHeight = 160, ariaLabel, readOnly = false }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const [editability] = useState(() => new Compartment());
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!host.current) return;
    const state = EditorState.create({
      doc: value,
      extensions: [
        editability.of([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]),
        lineNumbers(),
        history(),
        bracketMatching(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        syntaxHighlighting(themedHighlight),
        language === "json" ? json() : StreamLanguage.define(toml),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "aria-label": ariaLabel ?? language }),
        EditorView.theme({
          "&": { fontSize: "12.5px", border: "1px solid hsl(var(--input))", borderRadius: "8px", backgroundColor: "hsl(var(--card))", color: "hsl(var(--foreground))" },
          ".cm-scroller": { fontFamily: '"SF Mono", ui-monospace, Menlo, monospace', minHeight: `${minHeight}px`, maxHeight: "340px" },
          ".cm-content": { caretColor: "hsl(var(--primary))" },
          ".cm-gutters": { backgroundColor: "hsl(var(--muted))", color: "hsl(var(--muted-foreground))", border: "none", borderRight: "1px solid hsl(var(--border))", borderRadius: "8px 0 0 8px" },
          ".cm-activeLineGutter": { backgroundColor: "transparent" },
          "&.cm-focused": { outline: "none", borderColor: "hsl(var(--ring))", boxShadow: "0 0 0 3px hsl(var(--ring) / 0.2)" },
          "&.cm-focused .cm-selectionBackground, .cm-selectionBackground": { backgroundColor: "hsl(var(--primary) / 0.18)" },
          ".cm-matchingBracket": { backgroundColor: "hsl(var(--primary) / 0.16)", outline: "none" },
        }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onChangeRef.current(update.state.doc.toString());
        }),
      ],
    });
    view.current = new EditorView({ state, parent: host.current });
    return () => {
      view.current?.destroy();
      view.current = null;
    };
    // 只在挂载时创建；外部改值走下面的 effect。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== value) {
      v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: value } });
    }
  }, [value]);

  useEffect(() => {
    view.current?.dispatch({ effects: editability.reconfigure([EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)]) });
  }, [readOnly, editability]);

  return <div ref={host} data-testid={`editor-${language}`} />;
}
