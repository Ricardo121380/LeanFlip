// 文字变化的“溶解 + 光扫”：旧字向上模糊消失，新字从下方浮现，再扫过一道光。
import { useEffect, useRef, useState } from "react";
import { prefersReducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";

interface Layer {
  key: number;
  text: string;
  leaving: boolean;
}

export function Morph({ text, className }: { text: string; className?: string }) {
  const [layers, setLayers] = useState<Layer[]>([{ key: 0, text, leaving: false }]);
  const next = useRef(1);
  const shown = useRef(text);

  useEffect(() => {
    if (shown.current === text) return;
    shown.current = text;
    const key = next.current++;
    setLayers((all) => [...all.filter((l) => !l.leaving).map((l) => ({ ...l, leaving: true })), { key, text, leaving: false }]);
    const timer = setTimeout(() => setLayers((all) => all.filter((l) => !l.leaving)), 260);
    return () => clearTimeout(timer);
  }, [text]);

  return (
    <span className={cn("morph", className)}>
      {layers.map((layer) =>
        layer.leaving ? (
          <span key={layer.key} className="morph-out" aria-hidden="true">{layer.text}</span>
        ) : (
          <span key={layer.key} className={cn(layer.key > 0 && "morph-in")} >{layer.text}{layer.key > 0 && <span className="morph-shine" aria-hidden="true">{layer.text}</span>}</span>
        ),
      )}
    </span>
  );
}

/** 数字从 0 跑到目标值；减少动态效果时直接显示。 */
export function CountUp({ value }: { value: number }) {
  const [shown, setShown] = useState(() => (canAnimate() ? 0 : value));
  useEffect(() => {
    if (!canAnimate()) {
      setShown(value);
      return;
    }
    const start = performance.now();
    let frame = 0;
    const tick = (now: number) => {
      const p = Math.min(1, (now - start) / 560);
      setShown(Math.round(value * (1 - Math.pow(1 - p, 3))));
      if (p < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [value]);
  return <>{shown}</>;
}

function canAnimate() {
  return typeof requestAnimationFrame === "function" && !prefersReducedMotion();
}
