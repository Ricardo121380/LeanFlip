import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { json } from "@codemirror/lang-json";
import { StreamLanguage, syntaxHighlighting, defaultHighlightStyle, bracketMatching } from "@codemirror/language";
import { toml } from "@codemirror/legacy-modes/mode/toml";

interface Props {
  value: string;
  onChange: (value: string) => void;
  language: "json" | "toml";
  minHeight?: number;
  ariaLabel?: string;
}

/** 纯文本编辑器：内容原样进出，不做任何格式化（TOML 一字不改地保存）。 */
export function CodeEditor({ value, onChange, language, minHeight = 160, ariaLabel }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!host.current) return;
    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        history(),
        bracketMatching(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        syntaxHighlighting(defaultHighlightStyle),
        language === "json" ? json() : StreamLanguage.define(toml),
        EditorView.lineWrapping,
        EditorView.contentAttributes.of({ "aria-label": ariaLabel ?? language }),
        EditorView.theme({
          "&": { fontSize: "12.5px", border: "1px solid hsl(var(--border))", borderRadius: "6px" },
          ".cm-scroller": { fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", minHeight: `${minHeight}px`, maxHeight: "340px" },
          "&.cm-focused": { outline: "2px solid hsl(var(--ring) / 0.4)" },
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

  return <div ref={host} data-testid={`editor-${language}`} />;
}
