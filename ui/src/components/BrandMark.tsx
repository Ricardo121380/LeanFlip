// 和 App 图标（crates/app/icons/leanflip.svg）同一个图形：亮的 L 和翻过来变暗的 L。颜色固定，深浅主题一致。
export function BrandMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 100 100" className={className} aria-hidden="true">
      <rect width="100" height="100" rx="23" fill="#4A5288" />
      <g fill="none" strokeWidth="9" strokeLinecap="round" strokeLinejoin="round">
        <path d="M42 30H70V74" stroke="#9AA0D0" />
        <path d="M30 26V70H58" stroke="#F3F4FC" />
      </g>
    </svg>
  );
}
