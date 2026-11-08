<script lang="ts">
  // オーバーレイを OBS のブラウザソースと同じ大きさで開き、枠に合わせて縮めて見せる。
  // 枠の下には、OBS に追加するソースの大きさを書く（中身に重ならないよう枠の外に出す）。
  // 操作パネルはダークなので、iframe の color-scheme を中身（ライト）に合わせないと背景が白く塗られる
  import type { Snippet } from "svelte";

  let {
    src,
    title,
    size = [1920, 1080],
    children,
  }: {
    src: string;
    title: string;
    size?: [number, number];
    /** 大きさの横（右）に並べるもの */
    children?: Snippet;
  } = $props();
  let width = $state(0);
</script>

<figure style:max-width="{size[0]}px" class="mx-auto flex w-full flex-col gap-1.5">
  <div
    style:aspect-ratio="{size[0]} / {size[1]}"
    class="relative w-full overflow-hidden rounded-lg border bg-[radial-gradient(ellipse_at_30%_20%,#3b4a5e,#11151c_70%)]"
    bind:clientWidth={width}
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
  <figcaption class="flex items-center justify-between gap-2">
    <span class="text-xs whitespace-nowrap text-muted-foreground tabular-nums">
      プレビュー　{size[0]} × {size[1]}
    </span>
    {@render children?.()}
  </figcaption>
</figure>
