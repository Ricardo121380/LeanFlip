import { useEffect, useMemo, useState, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQueries } from "@tanstack/react-query";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { Download, Layers, Search, Settings, UsersRound } from "lucide-react";
import { toast } from "sonner";
import { keys, useAction, useScenes, useSwitchProvider } from "@/hooks/queries";
import { useSceneActions } from "@/components/Scenes";
import { api, type ToolId } from "@/lib/api";
import { endpointOf } from "@/lib/cards";
import { TOOLS, type ToolMeta } from "@/lib/tools";
import { ToolGlyph } from "@/components/Glyph";
import { cn } from "@/lib/utils";

interface Item {
  key: string;
  group: string;
  label: string;
  detail?: string;
  hint: string;
  tool?: ToolMeta;
  icon?: ReactNode;
  search: string;
  run: () => unknown;
}

/** ⌘K / Ctrl+K：应用场景（先预览），搜索供应商直接切换或启用，也可以跳到任意工具和页面。 */
export function CommandPalette({ open, onOpenChange, onNavigate }: { open: boolean; onOpenChange: (open: boolean) => void; onNavigate: (hash: string) => boolean | void | Promise<boolean> }) {
  const { t } = useTranslation();
  const go = onNavigate;
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const results = useQueries({
    queries: TOOLS.map((tool) => ({ queryKey: keys.providers(tool.id), queryFn: () => api.listProviders(tool.id), enabled: open })),
  });
  const scenes = useScenes(open).data;
  const sceneActions = useSceneActions();
  const switchAction = useSwitchProvider();
  const toggleAction = useAction(
    ({ tool, id, enabled }: { tool: ToolId; id: string; name: string; enabled: boolean }) => api.setInLive(tool, id, enabled),
    (_r, { name, enabled }) => toast.success(t(enabled ? "card.enabled" : "card.disabled", { name })),
  );

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
    }
  }, [open]);

  const loading = results.some((r) => r.isLoading);
  const items = useMemo(() => {
    const out: Item[] = [];
    for (const scene of scenes?.scenes ?? []) {
      out.push({
        key: `scene:${scene.id}`,
        group: t("palette.scenes"),
        label: t("palette.sceneLabel", { name: scene.name }),
        hint: scene.status === "active" ? t("palette.current") : t("palette.apply"),
        icon: <Layers className="h-3.5 w-3.5" />,
        search: `${scene.name} ${t("palette.scenes")} scene`.toLocaleLowerCase(),
        run: () => sceneActions.apply(scene.id),
      });
    }
    TOOLS.forEach((tool, index) => {
      const data = results[index]?.data;
      if (!data) return;
      const current = data.providers.find((p) => p.id === data.current);
      for (const p of data.providers) {
        const detail = endpointOf(tool.id, p);
        const search = `${tool.label} ${tool.id} ${p.name} ${p.id} ${detail ?? ""}`.toLocaleLowerCase();
        if (data.kind === "switch") {
          const isCurrent = data.current === p.id;
          out.push({
            key: `switch:${tool.id}:${p.id}`,
            group: t("palette.providers"),
            label: `${tool.label} › ${p.name}`,
            detail,
            hint: isCurrent ? t("palette.current") : t("palette.switch"),
            tool,
            search,
            run: async () => {
              if (await go(`#tool=${tool.id}`) === false) return;
              if (!isCurrent) switchAction.mutate({ tool: tool.id, id: p.id, name: p.name, previous: current ? { id: current.id, name: current.name } : null });
            },
          });
        } else {
          const enabled = data.inLive.includes(p.id);
          out.push({
            key: `coexist:${tool.id}:${p.id}`,
            group: t("palette.coexist"),
            label: `${tool.label} › ${p.name}`,
            detail,
            hint: enabled ? t("palette.disable") : t("palette.enable"),
            tool,
            search,
            run: async () => {
              if (await go(`#tool=${tool.id}`) === false) return;
              if (!data.liveError) toggleAction.mutate({ tool: tool.id, id: p.id, name: p.name, enabled: !enabled });
            },
          });
        }
      }
    });
    for (const tool of TOOLS) {
      out.push({ key: `tool:${tool.id}`, group: t("palette.tools"), label: tool.label, hint: t("palette.open"), tool, search: `${tool.label} ${tool.id}`.toLocaleLowerCase(), run: () => go(`#tool=${tool.id}`) });
    }
    const pages: [string, string, ReactNode][] = [
      ["#codex", t("nav.codexAccounts"), <UsersRound key="codex" className="h-3.5 w-3.5" />],
      ["#import", t("nav.import"), <Download key="import" className="h-3.5 w-3.5" />],
      ["#settings", t("nav.settings"), <Settings key="settings" className="h-3.5 w-3.5" />],
    ];
    for (const [hash, label, icon] of pages) {
      out.push({ key: `page:${hash}`, group: t("palette.pages"), label, hint: t("palette.open"), icon, search: `${label} ${hash}`.toLocaleLowerCase(), run: () => go(hash) });
    }
    return out;
    // `results` is a new array each render; the data references inside it are stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [t, go, scenes, sceneActions, ...results.map((r) => r.data)]);

  const needle = query.trim().toLocaleLowerCase();
  const filtered = needle ? items.filter((item) => needle.split(/\s+/).every((word) => item.search.includes(word))) : items;
  const selected = Math.min(active, Math.max(0, filtered.length - 1));

  useEffect(() => {
    document.getElementById(`palette-option-${selected}`)?.scrollIntoView({ block: "nearest" });
  }, [selected, filtered.length]);

  const run = (item: Item | undefined) => {
    if (!item) return;
    onOpenChange(false);
    item.run();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive(Math.min(selected + 1, filtered.length - 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive(Math.max(selected - 1, 0));
    } else if (event.key === "Enter") {
      event.preventDefault();
      run(filtered[selected]);
    }
  };

  let lastGroup = "";
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-overlay/30 backdrop-blur-[2px] data-[state=open]:animate-in data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[12vh] z-50 w-[calc(100vw-2rem)] max-w-xl -translate-x-1/2 overflow-hidden rounded-xl border bg-card shadow-2xl shadow-overlay/25 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-[0.98]"
        >
          <DialogPrimitive.Title className="sr-only">{t("palette.title")}</DialogPrimitive.Title>
          <div className="flex items-center gap-2.5 border-b px-4 text-muted-foreground">
            <Search className="h-4 w-4 shrink-0" aria-hidden="true" />
            <input
              autoFocus
              role="combobox"
              aria-expanded="true"
              aria-controls="palette-list"
              aria-activedescendant={filtered.length ? `palette-option-${selected}` : undefined}
              aria-label={t("palette.placeholder")}
              placeholder={t("palette.placeholder")}
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setActive(0);
              }}
              onKeyDown={onKeyDown}
              className="h-12 min-w-0 flex-1 bg-transparent text-[15px] text-foreground outline-none placeholder:text-muted-foreground focus-visible:outline-none"
            />
          </div>
          <div id="palette-list" role="listbox" aria-label={t("palette.title")} className="max-h-[min(60vh,380px)] overflow-y-auto p-1.5">
            {filtered.length === 0 ? (
              <p className="px-3 py-6 text-center text-sm text-muted-foreground" role="status">{loading ? t("palette.loading") : t("palette.empty")}</p>
            ) : (
              filtered.map((item, index) => {
                const header = item.group !== lastGroup ? item.group : null;
                lastGroup = item.group;
                return (
                  <div key={item.key}>
                    {header && <div className="px-2.5 pb-1 pt-2.5 text-[11px] font-medium text-muted-foreground" aria-hidden="true">{header}</div>}
                    <div
                      id={`palette-option-${index}`}
                      role="option"
                      aria-selected={index === selected}
                      onMouseMove={() => index !== selected && setActive(index)}
                      onClick={() => run(item)}
                      className={cn("flex cursor-pointer items-center gap-3 rounded-md px-2.5 py-2 text-sm", index === selected && "bg-accent")}
                    >
                      {item.icon ? (
                        <span className="grid w-5 place-items-center text-muted-foreground">{item.icon}</span>
                      ) : (
                        item.tool && <ToolGlyph tool={item.tool} size={20} />
                      )}
                      <span className="min-w-0 flex-1 truncate">
                        {item.label}
                        {item.detail && <span className="ml-2 font-mono text-xs text-muted-foreground">{item.detail}</span>}
                      </span>
                      <span className={cn("shrink-0 text-xs text-muted-foreground", index === selected && "font-medium text-primary")}>{item.hint}</span>
                    </div>
                  </div>
                );
              })
            )}
          </div>
          <div className="flex gap-4 border-t bg-muted/60 px-4 py-2 text-[11px] text-muted-foreground">
            <span>{t("palette.navigate")}</span>
            <span>{t("palette.run")}</span>
            <span>{t("palette.close")}</span>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
