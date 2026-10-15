// 首页标题区的场景下拉：按钮上是当前场景（手动改过后标「已偏离」），菜单里点场景先预览再应用。
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Layers, Plus, Settings2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSceneActions } from "@/components/Scenes";
import { useScenes } from "@/hooks/queries";
import { currentScene } from "@/lib/scenes";
import { cn } from "@/lib/utils";

const ITEM = "flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:bg-accent";

export function SceneMenu() {
  const { t } = useTranslation();
  const scenes = useScenes();
  const actions = useSceneActions();
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const list = scenes.data?.scenes ?? [];
  const current = currentScene(scenes.data);

  const items = () => [...(root.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
  const close = (focus = true) => {
    setOpen(false);
    if (focus) button.current?.focus();
  };

  useEffect(() => {
    if (!open) return;
    items()[0]?.focus();
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  const onMenuKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const all = items();
    const index = all.indexOf(document.activeElement as HTMLElement);
    const move = (next: number) => {
      event.preventDefault();
      all[(next + all.length) % all.length]?.focus();
    };
    if (event.key === "ArrowDown") move(index + 1);
    else if (event.key === "ArrowUp") move(index - 1);
    else if (event.key === "Home") move(0);
    else if (event.key === "End") move(all.length - 1);
    else if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "Tab") setOpen(false);
  };

  const run = (action: () => void) => {
    close(false);
    action();
  };

  const label = current ? t(current.drifted ? "scene.menuDrifted" : "scene.menuLabel", { name: current.scene.name }) : t("scene.menu");

  return (
    <div ref={root} className="relative">
      <Button
        ref={button}
        size="sm"
        variant="outline"
        className="h-8 max-w-[16rem]"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={label}
        title={current?.drifted ? t("scene.driftedHint", { name: current.scene.name }) : undefined}
        onClick={() => setOpen((value) => !value)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" && !open) {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        <Layers aria-hidden="true" />
        <span className="truncate">{current ? current.scene.name : t("scene.menu")}</span>
        {current?.drifted && <span className="text-xs font-normal text-warning">· {t("scene.drifted")}</span>}
        <ChevronDown className="text-muted-foreground" aria-hidden="true" />
      </Button>
      {open && (
        <div
          role="menu"
          aria-label={t("scene.menu")}
          onKeyDown={onMenuKey}
          className="absolute right-0 top-full z-40 mt-1.5 w-72 rounded-lg border bg-card p-1 shadow-lg shadow-overlay/15 animate-in fade-in-0 zoom-in-[0.98]"
        >
          {list.length === 0 ? (
            <p className="px-2.5 py-2 text-xs leading-5 text-muted-foreground">{t("scene.none")}</p>
          ) : (
            list.map((scene) => {
              const active = scene.status === "active";
              const note = scene.missing.length > 0 ? t("scene.incomplete") : scene.status === "drifted" ? t("scene.drifted") : null;
              return (
                <button
                  key={scene.id}
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  className={ITEM}
                  onClick={() => run(() => actions.apply(scene.id))}
                >
                  <Check className={cn("h-4 w-4 shrink-0 text-primary", !active && "invisible")} aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate">{scene.name}</span>
                  {active && <span className="sr-only">{t("scene.active")}</span>}
                  {note && <span className={cn("shrink-0 text-xs", scene.missing.length > 0 ? "text-warning" : "text-muted-foreground")}>{note}</span>}
                </button>
              );
            })
          )}
          <div className="my-1 h-px bg-border" role="separator" />
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            className={ITEM}
            onClick={() => run(() => actions.manage({ create: true }))}
          >
            <Plus className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
            {t("scene.saveCurrent")}
          </button>
          {list.length > 0 && (
            <button
              type="button"
              role="menuitem"
              tabIndex={-1}
              className={ITEM}
              onClick={() => run(() => actions.manage())}
            >
              <Settings2 className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
              {t("scene.manage")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
