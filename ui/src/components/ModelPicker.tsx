// 模型名输入框：可以直接输入，也可以从获取到的模型列表里挑选。
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import { Input } from "@/components/ui/input";
import type { RemoteModel } from "@/lib/api";
import { cn } from "@/lib/utils";

const LIMIT = 200;

interface Props {
  id: string;
  value: string;
  onChange: (value: string) => void;
  /** 选中一项时调用；不传则等同 onChange。 */
  onPick?: (id: string) => void;
  suggestions: RemoteModel[];
  placeholder?: string;
  disabled?: boolean;
  onEnter?: () => void;
}

export function ModelPicker({ id, value, onChange, onPick, suggestions, placeholder, disabled, onEnter }: Props) {
  const { t } = useTranslation();
  const listId = useId();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const [active, setActive] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  const matches = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const list = q ? suggestions.filter((m) => m.id.toLowerCase().includes(q) || (m.name ?? "").toLowerCase().includes(q)) : suggestions;
    return list.slice(0, LIMIT);
  }, [filter, suggestions]);
  const expanded = open && matches.length > 0 && !disabled;
  // 靠近滚动区底部时，把整个下拉滚进可见范围。
  useEffect(() => {
    if (expanded) list.current?.scrollIntoView?.({ block: "nearest" });
  }, [expanded]);
  useEffect(() => {
    if (expanded) list.current?.children[active]?.scrollIntoView?.({ block: "nearest" });
  }, [active, expanded]);

  const pick = (model: RemoteModel) => {
    (onPick ?? onChange)(model.id);
    setOpen(false);
    setFilter("");
  };
  const show = (nextFilter: string) => {
    setFilter(nextFilter);
    setActive(0);
    setOpen(true);
  };

  return (
    <div className="relative">
      <Input
        ref={input}
        id={id}
        value={value}
        disabled={disabled}
        spellCheck={false}
        autoComplete="off"
        placeholder={placeholder}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={expanded}
        aria-controls={expanded ? listId : undefined}
        aria-activedescendant={expanded ? `${listId}-${active}` : undefined}
        className={cn("font-mono text-[13px]", suggestions.length > 0 && "pr-9")}
        onChange={(e) => {
          onChange(e.target.value);
          if (suggestions.length) show(e.target.value);
        }}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            if (!suggestions.length) return;
            e.preventDefault();
            if (!expanded) return show("");
            setActive((i) => (i + (e.key === "ArrowDown" ? 1 : -1) + matches.length) % matches.length);
          } else if (e.key === "Enter") {
            e.preventDefault();
            if (expanded && matches[active]) pick(matches[active]);
            else onEnter?.();
          } else if (e.key === "Escape" && expanded) {
            setOpen(false);
          }
        }}
      />
      {suggestions.length > 0 && (
        <button
          type="button"
          tabIndex={-1}
          disabled={disabled}
          aria-label={t("editor.showModels")}
          className="absolute right-0.5 top-0.5 grid h-8 w-8 place-items-center rounded-md text-muted-foreground hover:text-foreground disabled:opacity-50"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            input.current?.focus();
            if (expanded) setOpen(false);
            else show("");
          }}
        >
          <ChevronDown className={cn("h-4 w-4 transition-transform", expanded && "rotate-180")} />
        </button>
      )}
      {expanded && (
        <ul ref={list} id={listId} role="listbox" className="absolute left-0 right-0 top-full z-30 mt-1 max-h-60 overflow-y-auto rounded-md border bg-card p-1 shadow-lg shadow-overlay/10">
          {matches.map((model, index) => (
            <li
              key={model.id}
              id={`${listId}-${index}`}
              role="option"
              aria-selected={index === active}
              className={cn(
                "flex cursor-default items-baseline justify-between gap-3 rounded px-2 py-1.5 text-sm",
                index === active && "bg-accent",
                model.id === value && "font-medium",
              )}
              onMouseDown={(e) => e.preventDefault()}
              onMouseEnter={() => setActive(index)}
              onClick={() => pick(model)}
            >
              <span className="min-w-0 break-all font-mono text-[13px]">{model.id}</span>
              {model.name && model.name !== model.id && <span className="shrink-0 truncate text-xs text-muted-foreground">{model.name}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
