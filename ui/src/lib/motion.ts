// 动效的公共部分：减少动态效果、视图过渡、按 key 的位移动画（FLIP）。
import { useLayoutEffect, useRef, type RefObject } from "react";
import { flushSync } from "react-dom";

export function prefersReducedMotion(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** 能用 Web Animations 且没有要求减少动态效果。测试环境（jsdom）里为 false。 */
export function canAnimate(): boolean {
  return typeof Element !== "undefined" && typeof Element.prototype.animate === "function" && !prefersReducedMotion();
}

type TransitionDocument = Document & { startViewTransition?: (update: () => void) => unknown };

/** 用浏览器的视图过渡包住一次界面切换；不支持或减少动态效果时直接切换。 */
export function withViewTransition(update: () => void) {
  const doc = document as TransitionDocument;
  if (!doc.startViewTransition || prefersReducedMotion()) {
    update();
    return;
  }
  doc.startViewTransition(() => flushSync(update));
}

interface Snapshot {
  x: number;
  y: number;
  w: number;
  text: string;
  node: HTMLElement;
}

const SPRING = "cubic-bezier(.2,1.12,.35,1)";
const SETTLE = "cubic-bezier(.2,.8,.2,1)";

/**
 * 内容按 `data-k` 标记的子元素，在 `signature` 变化时：留下的从旧位置滑到新位置，
 * 新出现的模糊浮现，消失的在 `ghosts` 层里溶解。只在签名变化时动，窗口缩放不触发。
 */
export function useFlip(box: RefObject<HTMLElement>, ghosts: RefObject<HTMLElement>, signature: string, stagger = false) {
  const last = useRef<Map<string, Snapshot> | null>(null);
  const lastSignature = useRef(signature);

  useLayoutEffect(() => {
    const root = box.current;
    if (!root) return;
    const take = () => {
      const origin = root.getBoundingClientRect();
      const map = new Map<string, Snapshot>();
      root.querySelectorAll<HTMLElement>("[data-k]").forEach((node) => {
        const rect = node.getBoundingClientRect();
        map.set(node.dataset.k!, { x: rect.left - origin.left, y: rect.top - origin.top, w: rect.width, text: node.textContent ?? "", node });
      });
      return map;
    };
    const before = last.current;
    const now = take();
    if (before && lastSignature.current !== signature && canAnimate()) {
      const kept = new Set<string>();
      let nth = 0;
      now.forEach((next, key) => {
        const prev = before.get(key);
        // 文字变了的非按钮元素按“换掉”处理：旧的溶解，新的浮现。
        const replaced = prev && next.node.tagName !== "BUTTON" && next.node.tagName !== "A" && prev.text !== next.text;
        if (prev && !replaced) {
          kept.add(key);
          const dx = prev.x - next.x;
          const dy = prev.y - next.y;
          if (Math.abs(dx) + Math.abs(dy) > 0.5) next.node.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: "none" }], { duration: 480, easing: SPRING });
          return;
        }
        next.node.animate(
          [{ opacity: 0, transform: "translateX(-10px)", filter: "blur(5px)" }, { opacity: 1, transform: "none", filter: "blur(0px)" }],
          { duration: 340, delay: stagger ? 30 + nth++ * 40 : 150, easing: SETTLE, fill: "backwards" },
        );
      });
      const layer = ghosts.current;
      if (layer) {
        const origin = layer.getBoundingClientRect();
        const offset = root.getBoundingClientRect();
        before.forEach((prev, key) => {
          if (kept.has(key)) return;
          const ghost = prev.node.cloneNode(true) as HTMLElement;
          ghost.removeAttribute("data-k");
          ghost.removeAttribute("id");
          ghost.setAttribute("aria-hidden", "true");
          ghost.tabIndex = -1;
          Object.assign(ghost.style, {
            position: "absolute",
            left: `${prev.x + offset.left - origin.left}px`,
            top: `${prev.y + offset.top - origin.top}px`,
            width: `${prev.w}px`,
            margin: "0",
            pointerEvents: "none",
          });
          layer.append(ghost);
          ghost
            .animate([{ opacity: 1, filter: "blur(0px)" }, { opacity: 0, transform: "translateY(-8px)", filter: "blur(5px)" }], { duration: 240, easing: "ease-in", fill: "forwards" })
            .finished.then(() => ghost.remove(), () => ghost.remove());
        });
      }
    }
    lastSignature.current = signature;
    // Clones keep the old words around for the next dissolve.
    now.forEach((snap) => { snap.node = snap.node.cloneNode(true) as HTMLElement; });
    last.current = now;
  });
}
