<script lang="ts">
  // オーバーレイを OBS のブラウザソースと同じ大きさで開き、枠に合わせて縮めて見せる。
  // 操作パネルはダークなので、iframe の color-scheme を中身（ライト）に合わせないと背景が白く塗られる
  let {
    src,
    title,
    size = [1920, 1080],
  }: { src: string; title: string; size?: [number, number] } = $props();
  let width = $state(0);
</script>

<div
  style:aspect-ratio="{size[0]} / {size[1]}"
  style:max-width="{size[0]}px"
  class="relative w-full overflow-hidden rounded-lg border bg-[radial-gradient(ellipse_at_30%_20%,#3b4a5e,#11151c_70%)]"
  bind:clientWidth={width}
>
  <span class="absolute right-3 bottom-2 text-[10px] tracking-wide text-white/40 uppercase"
    >プレビュー</span
  >
  <iframe
    {src}
    {title}
    class="pointer-events-none absolute top-0 left-0 origin-top-left border-0"
    width={size[0]}
    height={size[1]}
    style:transform="scale({width / size[0]})"
    style:color-scheme="light"
  ></iframe>
</div>
