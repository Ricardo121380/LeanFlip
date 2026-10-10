import "@testing-library/jest-dom/vitest";
import i18n from "@/i18n";

void i18n.changeLanguage("zh");

class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
// jsdom 没有这些；Radix / CodeMirror 会用到。
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;
if (!document.createRange().getClientRects) {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}
Element.prototype.scrollIntoView ??= () => {};
Element.prototype.hasPointerCapture ??= () => false;
